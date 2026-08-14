use std::{collections::VecDeque, sync::mpsc, thread::JoinHandle};

use chrono::Local;

use crate::settings::AppSettings;

use super::{
    model::{
        LogEntry, LogEvent, ServiceActivity, ServiceError, ServiceNotice, ServiceSnapshot,
        format_error,
    },
    runtime::{WorkerCommand, WorkerEvent, WorkerSnapshot, service_worker},
};

pub struct ServiceController {
    command_tx: mpsc::Sender<WorkerCommand>,
    event_rx: mpsc::Receiver<WorkerEvent>,
    worker: Option<JoinHandle<()>>,
    snapshot: ServiceSnapshot,
    logs: VecDeque<LogEntry>,
    notices: VecDeque<ServiceNotice>,
    poll_pending: bool,
}

impl Default for ServiceController {
    fn default() -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || service_worker(command_rx, event_tx));

        Self {
            command_tx,
            event_rx,
            worker: Some(worker),
            snapshot: ServiceSnapshot::default(),
            logs: VecDeque::new(),
            notices: VecDeque::new(),
            poll_pending: false,
        }
    }
}

impl ServiceController {
    pub fn start(&mut self, settings: &AppSettings) {
        self.pump();
        if self.snapshot.running || self.snapshot.is_busy() {
            return;
        }

        self.snapshot.activity = ServiceActivity::Starting;
        if self
            .command_tx
            .send(WorkerCommand::Start(settings.clone()))
            .is_err()
        {
            self.finish_with_channel_error(ServiceActivity::Starting);
        }
    }

    pub fn stop(&mut self) {
        self.pump();
        if !self.snapshot.running || self.snapshot.is_busy() {
            return;
        }

        self.snapshot.activity = ServiceActivity::Stopping;
        if self.command_tx.send(WorkerCommand::Stop).is_err() {
            self.finish_with_channel_error(ServiceActivity::Stopping);
        }
    }

    pub fn disconnect_device(&mut self) {
        self.pump();
        if !self.snapshot.running || self.snapshot.is_busy() {
            return;
        }

        self.snapshot.activity = ServiceActivity::Disconnecting;
        if self.command_tx.send(WorkerCommand::Disconnect).is_err() {
            self.finish_with_channel_error(ServiceActivity::Disconnecting);
        }
    }

    pub fn refresh(&mut self) {
        self.pump();
        if self.poll_pending || self.snapshot.is_busy() {
            return;
        }
        self.poll_pending = true;
        if self.command_tx.send(WorkerCommand::Poll).is_err() {
            self.poll_pending = false;
        }
    }

    pub fn pump(&mut self) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                WorkerEvent::Snapshot(state) => {
                    let state = *state;
                    self.snapshot.running = state.running;
                    self.snapshot.info = state.info;
                    self.snapshot.status = state.status;
                    self.snapshot.status_error = state.status_error;
                }
                WorkerEvent::Log(event) => self.push_log(event),
                WorkerEvent::PollFinished => self.poll_pending = false,
                WorkerEvent::ActivityFinished { activity, result } => {
                    self.snapshot.activity = ServiceActivity::Idle;
                    match result {
                        Ok(()) => match activity {
                            ServiceActivity::Starting => {
                                self.notices.push_back(ServiceNotice::Started)
                            }
                            ServiceActivity::Stopping => {
                                self.notices.push_back(ServiceNotice::Stopped)
                            }
                            ServiceActivity::Disconnecting => {
                                self.notices.push_back(ServiceNotice::Disconnected)
                            }
                            ServiceActivity::Idle => {}
                        },
                        Err(error) => {
                            if activity == ServiceActivity::Starting {
                                self.push_log(LogEvent::StartFailed(error.clone()));
                            } else {
                                self.push_log(LogEvent::RequestFailed(format_error(&error)));
                            }
                            self.notices.push_back(ServiceNotice::Failed(error));
                        }
                    }
                }
            }
        }
        self.sync_logs();
    }

    pub fn snapshot(&self) -> &ServiceSnapshot {
        &self.snapshot
    }

    pub fn take_notice(&mut self) -> Option<ServiceNotice> {
        self.notices.pop_front()
    }

    pub fn push_log(&mut self, event: LogEvent) {
        if self.logs.len() >= MAX_LOG_ENTRIES {
            self.logs.pop_back();
        }
        self.logs.push_front(LogEntry {
            timestamp: timestamp(),
            event,
        });
        self.sync_logs();
    }

    pub fn clear_logs(&mut self) {
        self.logs.clear();
        self.sync_logs();
    }

    pub fn shutdown(&mut self) {
        let Some(worker) = self.worker.take() else {
            return;
        };
        let _ = self.command_tx.send(WorkerCommand::Shutdown);
        let _ = worker.join();
        self.snapshot.running = false;
        self.snapshot.info = None;
        self.snapshot.status = None;
        self.snapshot.activity = ServiceActivity::Idle;
    }

    fn finish_with_channel_error(&mut self, activity: ServiceActivity) {
        self.snapshot.activity = ServiceActivity::Idle;
        let error = ServiceError::Request("service worker is unavailable".to_string());
        if activity == ServiceActivity::Starting {
            self.push_log(LogEvent::StartFailed(error.clone()));
        } else {
            self.push_log(LogEvent::RequestFailed(format_error(&error)));
        }
        self.notices.push_back(ServiceNotice::Failed(error));
    }

    fn sync_logs(&mut self) {
        self.snapshot.logs = self.logs.iter().cloned().collect();
    }
}

impl Drop for ServiceController {
    fn drop(&mut self) {
        self.shutdown();
    }
}

const MAX_LOG_ENTRIES: usize = 80;

fn timestamp() -> String {
    Local::now().format("%H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::mpsc};

    use super::{MAX_LOG_ENTRIES, ServiceController, WorkerCommand, WorkerEvent, WorkerSnapshot};
    use crate::{
        service::{LogEvent, ServiceActivity, ServiceError, ServiceInfo, ServiceNotice},
        settings::AppSettings,
    };

    fn controller_with_channels() -> (ServiceController, mpsc::Receiver<WorkerCommand>) {
        let (command_tx, command_rx) = mpsc::channel();
        let (_event_tx, event_rx) = mpsc::channel();
        (
            ServiceController {
                command_tx,
                event_rx,
                worker: None,
                snapshot: Default::default(),
                logs: VecDeque::new(),
                notices: VecDeque::new(),
                poll_pending: false,
            },
            command_rx,
        )
    }

    fn controller_with_events() -> (ServiceController, mpsc::Sender<WorkerEvent>) {
        let (command_tx, _command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        (
            ServiceController {
                command_tx,
                event_rx,
                worker: None,
                snapshot: Default::default(),
                logs: VecDeque::new(),
                notices: VecDeque::new(),
                poll_pending: false,
            },
            event_tx,
        )
    }

    fn service_info() -> ServiceInfo {
        serde_json::from_str(
            r#"{"event":"ready","control_url":"http://127.0.0.1:4100","control_port":4100,"discovery_port":41000,"pin":"123456","audio":{"sample_rate":48000,"channels":2,"sample_format":"s16le","packet_ms":5,"payload_type":96,"ssrc":1},"source":"tone","direct_target":null}"#,
        )
        .unwrap()
    }

    #[test]
    fn push_log_keeps_newest_entries_first_and_caps_history() {
        let mut controller = ServiceController::default();

        for index in 0..(MAX_LOG_ENTRIES + 3) {
            controller.push_log(LogEvent::ServiceOutput(format!("line-{index}")));
        }

        let logs = &controller.snapshot().logs;
        assert_eq!(logs.len(), MAX_LOG_ENTRIES);
        assert!(matches!(
            logs.first().map(|entry| &entry.event),
            Some(LogEvent::ServiceOutput(line)) if line == "line-82"
        ));
        assert!(matches!(
            logs.last().map(|entry| &entry.event),
            Some(LogEvent::ServiceOutput(line)) if line == "line-3"
        ));
    }

    #[test]
    fn clear_logs_syncs_snapshot() {
        let mut controller = ServiceController::default();
        controller.push_log(LogEvent::ServiceOutput("line".to_string()));

        controller.clear_logs();

        assert!(controller.snapshot().logs.is_empty());
    }

    #[test]
    fn start_stop_disconnect_and_refresh_send_worker_commands() {
        let (mut controller, command_rx) = controller_with_channels();

        controller.start(&AppSettings::default());
        assert_eq!(controller.snapshot.activity, ServiceActivity::Starting);
        assert!(matches!(
            command_rx.recv().unwrap(),
            WorkerCommand::Start(_)
        ));

        controller.snapshot.activity = ServiceActivity::Idle;
        controller.snapshot.running = true;
        controller.stop();
        assert_eq!(controller.snapshot.activity, ServiceActivity::Stopping);
        assert!(matches!(command_rx.recv().unwrap(), WorkerCommand::Stop));

        controller.snapshot.activity = ServiceActivity::Idle;
        controller.snapshot.running = true;
        controller.disconnect_device();
        assert_eq!(controller.snapshot.activity, ServiceActivity::Disconnecting);
        assert!(matches!(
            command_rx.recv().unwrap(),
            WorkerCommand::Disconnect
        ));

        controller.snapshot.activity = ServiceActivity::Idle;
        controller.refresh();
        assert!(controller.poll_pending);
        assert!(matches!(command_rx.recv().unwrap(), WorkerCommand::Poll));
    }

    #[test]
    fn pump_applies_snapshot_logs_and_poll_completion() {
        let (mut controller, event_tx) = controller_with_events();
        controller.poll_pending = true;

        event_tx
            .send(WorkerEvent::Snapshot(Box::new(WorkerSnapshot {
                running: true,
                info: Some(service_info()),
                status: None,
                status_error: Some("offline".to_string()),
            })))
            .unwrap();
        event_tx
            .send(WorkerEvent::Log(LogEvent::Ready(
                "http://127.0.0.1:4100".to_string(),
            )))
            .unwrap();
        event_tx.send(WorkerEvent::PollFinished).unwrap();

        controller.pump();

        assert!(controller.snapshot.running);
        assert_eq!(
            controller
                .snapshot
                .info
                .as_ref()
                .map(|info| info.control_url.as_str()),
            Some("http://127.0.0.1:4100")
        );
        assert_eq!(controller.snapshot.status_error.as_deref(), Some("offline"));
        assert!(!controller.poll_pending);
        assert!(matches!(
            controller.snapshot.logs.first().map(|entry| &entry.event),
            Some(LogEvent::Ready(url)) if url == "http://127.0.0.1:4100"
        ));
    }

    #[test]
    fn pump_turns_successful_activity_results_into_notices() {
        let (mut controller, event_tx) = controller_with_events();

        event_tx
            .send(WorkerEvent::ActivityFinished {
                activity: ServiceActivity::Starting,
                result: Ok(()),
            })
            .unwrap();
        event_tx
            .send(WorkerEvent::ActivityFinished {
                activity: ServiceActivity::Stopping,
                result: Ok(()),
            })
            .unwrap();
        event_tx
            .send(WorkerEvent::ActivityFinished {
                activity: ServiceActivity::Disconnecting,
                result: Ok(()),
            })
            .unwrap();

        controller.pump();

        assert!(matches!(
            controller.take_notice(),
            Some(ServiceNotice::Started)
        ));
        assert!(matches!(
            controller.take_notice(),
            Some(ServiceNotice::Stopped)
        ));
        assert!(matches!(
            controller.take_notice(),
            Some(ServiceNotice::Disconnected)
        ));
        assert_eq!(controller.snapshot.activity, ServiceActivity::Idle);
    }

    #[test]
    fn pump_logs_failed_start_and_request_activities() {
        let (mut controller, event_tx) = controller_with_events();

        event_tx
            .send(WorkerEvent::ActivityFinished {
                activity: ServiceActivity::Starting,
                result: Err(ServiceError::NotFound),
            })
            .unwrap();
        event_tx
            .send(WorkerEvent::ActivityFinished {
                activity: ServiceActivity::Disconnecting,
                result: Err(ServiceError::Request("offline".to_string())),
            })
            .unwrap();

        controller.pump();

        assert!(matches!(
            controller.snapshot.logs.get(1).map(|entry| &entry.event),
            Some(LogEvent::StartFailed(ServiceError::NotFound))
        ));
        assert!(matches!(
            controller.snapshot.logs.first().map(|entry| &entry.event),
            Some(LogEvent::RequestFailed(message)) if message == "offline"
        ));
        assert!(matches!(
            controller.take_notice(),
            Some(ServiceNotice::Failed(ServiceError::NotFound))
        ));
        assert!(matches!(
            controller.take_notice(),
            Some(ServiceNotice::Failed(ServiceError::Request(message))) if message == "offline"
        ));
    }

    #[test]
    fn channel_error_finishes_activity_and_queues_failure_notice() {
        let mut controller = ServiceController::default();

        controller.finish_with_channel_error(ServiceActivity::Disconnecting);

        assert_eq!(controller.snapshot().activity, ServiceActivity::Idle);
        assert!(controller.take_notice().is_some());
        assert!(matches!(
            controller.snapshot().logs.first().map(|entry| &entry.event),
            Some(LogEvent::RequestFailed(_))
        ));
    }

    #[test]
    fn refresh_is_ignored_while_busy() {
        let mut controller = ServiceController::default();
        controller.snapshot.activity = ServiceActivity::Starting;

        controller.refresh();

        assert!(!controller.poll_pending);
        assert_eq!(controller.snapshot.activity, ServiceActivity::Starting);
    }

    #[test]
    fn stop_is_ignored_when_service_is_not_running() {
        let mut controller = ServiceController::default();

        controller.stop();

        assert_eq!(controller.snapshot.activity, ServiceActivity::Idle);
    }
}
