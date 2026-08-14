use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc,
};

use crate::settings::AppSettings;

use super::{
    http::{disconnect_device, fetch_status},
    model::{
        LogEvent, ServiceActivity, ServiceError, ServiceInfo, ServiceSnapshot, StatusResponse,
    },
    process::{find_service_path, terminate_child, wait_for_ready},
};

pub(super) enum WorkerCommand {
    Start(AppSettings),
    Stop,
    Poll,
    Disconnect,
    Shutdown,
}

pub(super) enum WorkerEvent {
    Snapshot(Box<WorkerSnapshot>),
    Log(LogEvent),
    PollFinished,
    ActivityFinished {
        activity: ServiceActivity,
        result: Result<(), ServiceError>,
    },
}

#[derive(Clone, Default)]
pub(super) struct WorkerSnapshot {
    pub(super) running: bool,
    pub(super) info: Option<ServiceInfo>,
    pub(super) status: Option<StatusResponse>,
    pub(super) status_error: Option<String>,
}

#[derive(Default)]
struct ServiceRuntime {
    child: Option<Child>,
    info: Option<ServiceInfo>,
    status: Option<StatusResponse>,
    status_error: Option<String>,
}

pub(super) fn service_worker(
    command_rx: mpsc::Receiver<WorkerCommand>,
    event_tx: mpsc::Sender<WorkerEvent>,
) {
    let mut runtime = ServiceRuntime::default();
    send_snapshot(&runtime, &event_tx);

    while let Ok(command) = command_rx.recv() {
        match command {
            WorkerCommand::Start(settings) => {
                let result = runtime.start(&settings, &event_tx);
                send_snapshot(&runtime, &event_tx);
                let _ = event_tx.send(WorkerEvent::ActivityFinished {
                    activity: ServiceActivity::Starting,
                    result,
                });
            }
            WorkerCommand::Stop => {
                runtime.stop(&event_tx, true);
                send_snapshot(&runtime, &event_tx);
                let _ = event_tx.send(WorkerEvent::ActivityFinished {
                    activity: ServiceActivity::Stopping,
                    result: Ok(()),
                });
            }
            WorkerCommand::Poll => {
                runtime.poll(&event_tx);
                send_snapshot(&runtime, &event_tx);
                let _ = event_tx.send(WorkerEvent::PollFinished);
            }
            WorkerCommand::Disconnect => {
                let result = runtime.disconnect();
                if result.is_ok() {
                    let _ = event_tx.send(WorkerEvent::Log(LogEvent::DeviceDisconnected));
                    runtime.poll(&event_tx);
                }
                send_snapshot(&runtime, &event_tx);
                let _ = event_tx.send(WorkerEvent::ActivityFinished {
                    activity: ServiceActivity::Disconnecting,
                    result,
                });
            }
            WorkerCommand::Shutdown => {
                runtime.stop(&event_tx, false);
                break;
            }
        }
    }
}

impl ServiceRuntime {
    fn start(
        &mut self,
        settings: &AppSettings,
        event_tx: &mpsc::Sender<WorkerEvent>,
    ) -> Result<(), ServiceError> {
        self.reap_child(event_tx);
        if self.child.is_some() {
            return Ok(());
        }

        let service = find_service_path()?;
        let mut child = Command::new(&service)
            .args(service_args(settings))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| ServiceError::Spawn {
                path: service.clone(),
                error: err.to_string(),
            })?;

        let Some(stdout) = child.stdout.take() else {
            terminate_child(&mut child);
            return Err(ServiceError::StdoutUnavailable);
        };

        if let Some(stderr) = child.stderr.take() {
            let stderr_tx = event_tx.clone();
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    let _ = stderr_tx.send(WorkerEvent::Log(LogEvent::ServiceOutput(line)));
                }
            });
        }

        let info = match wait_for_ready(stdout) {
            Ok(info) => info,
            Err(error) => {
                terminate_child(&mut child);
                return Err(error);
            }
        };

        let _ = event_tx.send(WorkerEvent::Log(LogEvent::ServicePath(
            service.display().to_string(),
        )));
        let _ = event_tx.send(WorkerEvent::Log(LogEvent::Ready(info.control_url.clone())));
        self.info = Some(info);
        self.child = Some(child);
        self.status = None;
        self.status_error = None;
        self.poll(event_tx);
        Ok(())
    }

    fn stop(&mut self, event_tx: &mpsc::Sender<WorkerEvent>, log: bool) {
        let was_running = self.child.is_some();
        if let Some(mut child) = self.child.take() {
            terminate_child(&mut child);
        }
        self.info = None;
        self.status = None;
        self.status_error = None;
        if log && was_running {
            let _ = event_tx.send(WorkerEvent::Log(LogEvent::Stopped));
        }
    }

    fn poll(&mut self, event_tx: &mpsc::Sender<WorkerEvent>) {
        self.reap_child(event_tx);
        let Some(info) = self.info.as_ref() else {
            self.status = None;
            self.status_error = None;
            return;
        };

        match fetch_status(info) {
            Ok(status) => {
                if self.status_error.take().is_some() {
                    let _ = event_tx.send(WorkerEvent::Log(LogEvent::StatusRestored));
                }
                self.status = Some(status);
            }
            Err(error) => {
                if self.status_error.as_deref() != Some(error.as_str()) {
                    let _ = event_tx.send(WorkerEvent::Log(LogEvent::StatusError(error.clone())));
                }
                self.status_error = Some(error);
            }
        }
    }

    fn disconnect(&mut self) -> Result<(), ServiceError> {
        let info = self
            .info
            .as_ref()
            .ok_or_else(|| ServiceError::Request("service is not running".to_string()))?;
        disconnect_device(info).map_err(ServiceError::Request)
    }

    fn reap_child(&mut self, event_tx: &mpsc::Sender<WorkerEvent>) {
        let Some(child) = self.child.as_mut() else {
            return;
        };

        match child.try_wait() {
            Ok(Some(status)) => {
                let _ = event_tx.send(WorkerEvent::Log(LogEvent::Exited(status.to_string())));
                self.child = None;
                self.info = None;
                self.status = None;
                self.status_error = None;
            }
            Ok(None) => {}
            Err(error) => {
                let _ = event_tx.send(WorkerEvent::Log(LogEvent::StatusError(error.to_string())));
                self.child = None;
                self.info = None;
                self.status = None;
                self.status_error = None;
            }
        }
    }
}

fn service_args(settings: &AppSettings) -> Vec<String> {
    vec![
        "--control-port".to_string(),
        settings.control_port_start.to_string(),
        "--control-port-end".to_string(),
        settings.control_port_end.to_string(),
        "--discovery-port".to_string(),
        settings.discovery_port_start.to_string(),
        "--discovery-port-end".to_string(),
        settings.discovery_port_end.to_string(),
        "--source".to_string(),
        settings.audio_source.as_str().to_string(),
        "--packet-ms".to_string(),
        settings.packet_ms.to_string(),
        "--json-events".to_string(),
    ]
}

fn send_snapshot(runtime: &ServiceRuntime, event_tx: &mpsc::Sender<WorkerEvent>) {
    let snapshot = ServiceSnapshot {
        running: runtime.child.is_some(),
        info: runtime.info.clone(),
        status: runtime.status.clone(),
        status_error: runtime.status_error.clone(),
        ..ServiceSnapshot::default()
    };
    let _ = event_tx.send(WorkerEvent::Snapshot(Box::new(WorkerSnapshot {
        running: snapshot.running,
        info: snapshot.info,
        status: snapshot.status,
        status_error: snapshot.status_error,
    })));
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::{
        ServiceInfo, ServiceRuntime, StatusResponse, WorkerCommand, WorkerEvent, send_snapshot,
        service_args, service_worker,
    };
    use crate::{
        service::{ServiceActivity, ServiceError},
        settings::{AppSettings, AudioSource},
    };

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn service_info() -> ServiceInfo {
        serde_json::from_str(
            r#"{"event":"ready","control_url":"http://127.0.0.1:4100","control_port":4100,"discovery_port":41000,"pin":"123456","audio":{"sample_rate":48000,"channels":2,"sample_format":"s16le","packet_ms":5,"payload_type":96,"ssrc":1},"source":"auto","direct_target":null}"#,
        )
        .unwrap()
    }

    fn status_response() -> StatusResponse {
        serde_json::from_str(
            r#"{"ok":true,"audio":{"sample_rate":48000,"channels":2,"sample_format":"s16le","packet_ms":5,"payload_type":96,"ssrc":1},"stats":{"target":null,"device":null,"media_source":"tone","packets_sent":2,"bytes_sent":960,"media_started_ms":5,"last_packet_at_ms":10}}"#,
        )
        .unwrap()
    }

    #[test]
    fn send_snapshot_reports_stopped_default_runtime() {
        let (tx, rx) = mpsc::channel();
        let runtime = ServiceRuntime::default();

        send_snapshot(&runtime, &tx);

        let WorkerEvent::Snapshot(snapshot) = rx.recv().unwrap() else {
            panic!("expected snapshot event");
        };
        assert!(!snapshot.running);
        assert!(snapshot.info.is_none());
        assert!(snapshot.status.is_none());
        assert!(snapshot.status_error.is_none());
    }

    #[test]
    fn send_snapshot_copies_cached_service_state() {
        let (tx, rx) = mpsc::channel();
        let runtime = ServiceRuntime {
            child: None,
            info: Some(service_info()),
            status: Some(status_response()),
            status_error: Some("offline".to_string()),
        };

        send_snapshot(&runtime, &tx);

        let WorkerEvent::Snapshot(snapshot) = rx.recv().unwrap() else {
            panic!("expected snapshot event");
        };
        assert!(!snapshot.running);
        assert_eq!(
            snapshot.info.as_ref().map(|info| info.control_url.as_str()),
            Some("http://127.0.0.1:4100")
        );
        assert_eq!(
            snapshot
                .status
                .as_ref()
                .map(|status| status.stats.packets_sent),
            Some(2)
        );
        assert_eq!(snapshot.status_error.as_deref(), Some("offline"));
    }

    #[test]
    fn disconnect_requires_running_service_info() {
        let mut runtime = ServiceRuntime::default();

        let error = runtime.disconnect().unwrap_err();

        assert!(
            matches!(error, ServiceError::Request(message) if message == "service is not running")
        );
    }

    #[test]
    fn poll_without_service_info_clears_stale_status() {
        let (tx, rx) = mpsc::channel();
        let mut runtime = ServiceRuntime {
            child: None,
            info: None,
            status: Some(status_response()),
            status_error: Some("offline".to_string()),
        };

        runtime.poll(&tx);

        assert!(runtime.status.is_none());
        assert!(runtime.status_error.is_none());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn stop_without_child_clears_cached_state_without_logging() {
        let (tx, rx) = mpsc::channel();
        let mut runtime = ServiceRuntime {
            child: None,
            info: Some(service_info()),
            status: Some(status_response()),
            status_error: Some("offline".to_string()),
        };

        runtime.stop(&tx, true);

        assert!(runtime.info.is_none());
        assert!(runtime.status.is_none());
        assert!(runtime.status_error.is_none());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn service_args_include_default_service_flags() {
        assert_eq!(
            service_args(&AppSettings::default()),
            strings(&[
                "--control-port",
                "4100",
                "--control-port-end",
                "4199",
                "--discovery-port",
                "41000",
                "--discovery-port-end",
                "41020",
                "--source",
                "auto",
                "--packet-ms",
                "5",
                "--json-events"
            ])
        );
    }

    #[test]
    fn service_args_reflect_custom_runtime_settings() {
        let settings = AppSettings {
            audio_source: AudioSource::Tone,
            packet_ms: 20,
            control_port_start: 5000,
            control_port_end: 5002,
            discovery_port_start: 5500,
            discovery_port_end: 5501,
            ..AppSettings::default()
        };

        assert_eq!(
            service_args(&settings),
            strings(&[
                "--control-port",
                "5000",
                "--control-port-end",
                "5002",
                "--discovery-port",
                "5500",
                "--discovery-port-end",
                "5501",
                "--source",
                "tone",
                "--packet-ms",
                "20",
                "--json-events"
            ])
        );
    }

    #[test]
    fn service_worker_emits_initial_snapshot_and_handles_shutdown() {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || service_worker(command_rx, event_tx));

        let WorkerEvent::Snapshot(snapshot) = event_rx.recv().unwrap() else {
            panic!("expected initial snapshot");
        };
        assert!(!snapshot.running);

        command_tx.send(WorkerCommand::Shutdown).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn service_worker_handles_stopped_stop_poll_and_disconnect_commands() {
        let (command_tx, command_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || service_worker(command_rx, event_tx));

        let WorkerEvent::Snapshot(initial) = event_rx.recv().unwrap() else {
            panic!("expected initial snapshot");
        };
        assert!(!initial.running);

        command_tx.send(WorkerCommand::Stop).unwrap();
        let WorkerEvent::Snapshot(stopped) = event_rx.recv().unwrap() else {
            panic!("expected stop snapshot");
        };
        assert!(!stopped.running);
        let WorkerEvent::ActivityFinished { activity, result } = event_rx.recv().unwrap() else {
            panic!("expected stop activity");
        };
        assert_eq!(activity, ServiceActivity::Stopping);
        assert!(result.is_ok());

        command_tx.send(WorkerCommand::Poll).unwrap();
        let WorkerEvent::Snapshot(polled) = event_rx.recv().unwrap() else {
            panic!("expected poll snapshot");
        };
        assert!(!polled.running);
        assert!(matches!(
            event_rx.recv().unwrap(),
            WorkerEvent::PollFinished
        ));

        command_tx.send(WorkerCommand::Disconnect).unwrap();
        let WorkerEvent::Snapshot(disconnected) = event_rx.recv().unwrap() else {
            panic!("expected disconnect snapshot");
        };
        assert!(!disconnected.running);
        let WorkerEvent::ActivityFinished { activity, result } = event_rx.recv().unwrap() else {
            panic!("expected disconnect activity");
        };
        assert_eq!(activity, ServiceActivity::Disconnecting);
        assert!(
            matches!(result, Err(ServiceError::Request(message)) if message == "service is not running")
        );

        command_tx.send(WorkerCommand::Shutdown).unwrap();
        worker.join().unwrap();
    }
}
