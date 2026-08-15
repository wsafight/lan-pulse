use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
};

use anyhow::{Context, Result, anyhow};
use rtrb::{Consumer, Producer, PushError, RingBuffer};

use super::{
    signal::{CaptureSignalReceiver, CaptureSignalSender, capture_signal_pair},
    source::PcmSource,
};

pub(crate) struct CaptureStatus {
    stop_requested: AtomicBool,
    producer_finished: AtomicBool,
    dropped: AtomicU64,
    error: Mutex<Option<String>>,
}

impl CaptureStatus {
    fn new() -> Self {
        Self {
            stop_requested: AtomicBool::new(false),
            producer_finished: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
            error: Mutex::new(None),
        }
    }

    fn set_error(&self, message: String) {
        let mut error = self
            .error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if error.is_none() {
            *error = Some(message);
        }
    }

    fn error_message(&self) -> Option<String> {
        self.error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

#[derive(Clone)]
pub(crate) struct CaptureControl {
    status: Arc<CaptureStatus>,
    signal: CaptureSignalSender,
}

impl CaptureControl {
    pub(crate) fn stop_requested(&self) -> bool {
        self.status.stop_requested.load(Ordering::Acquire)
    }

    pub(crate) fn fail(&self, message: String) {
        self.status.set_error(message);
    }

    pub(crate) fn error_message(&self) -> Option<String> {
        self.status.error_message()
    }

    pub(super) fn record_dropped(&self, count: u64) {
        self.status.dropped.fetch_add(count, Ordering::Relaxed);
    }

    fn finish(&self, error: Option<String>) {
        if let Some(error) = error {
            self.status.set_error(error);
        }
        self.status.producer_finished.store(true, Ordering::Release);
        self.signal.notify();
    }
}

pub(crate) struct CaptureProducer {
    frames: Producer<Vec<u8>>,
    recycled: Consumer<Vec<u8>>,
    spare: Option<Vec<u8>>,
    current: Option<Vec<u8>>,
    current_len: usize,
    packet_bytes: usize,
    pub(super) control: CaptureControl,
}

impl CaptureProducer {
    fn new(
        frames: Producer<Vec<u8>>,
        recycled: Consumer<Vec<u8>>,
        packet_bytes: usize,
        control: CaptureControl,
    ) -> Self {
        Self {
            frames,
            recycled,
            spare: None,
            current: None,
            current_len: 0,
            packet_bytes,
            control,
        }
    }

    pub(crate) fn fail(&self, message: String) {
        self.control.fail(message);
    }

    pub(crate) fn push_pcm(&mut self, mut pcm: &[u8]) {
        while !pcm.is_empty() {
            if self.current.is_none() {
                self.current = self.acquire_frame();
                self.current_len = 0;
            }
            let Some(frame) = self.current.as_mut() else {
                let dropped = pcm.len().div_ceil(self.packet_bytes) as u64;
                self.control.record_dropped(dropped);
                return;
            };
            let writable = (self.packet_bytes - self.current_len).min(pcm.len());
            frame[self.current_len..self.current_len + writable].copy_from_slice(&pcm[..writable]);
            self.current_len += writable;
            pcm = &pcm[writable..];

            if self.current_len == self.packet_bytes {
                let frame = self.current.take().expect("capture frame must exist");
                self.current_len = 0;
                self.submit_frame(frame);
            }
        }
    }

    pub(super) fn acquire_frame(&mut self) -> Option<Vec<u8>> {
        self.spare.take().or_else(|| self.recycled.pop().ok())
    }

    pub(super) fn submit_frame(&mut self, frame: Vec<u8>) {
        match self.frames.push(frame) {
            Ok(()) => self.control.signal.notify(),
            Err(PushError::Full(frame)) => {
                self.control.record_dropped(1);
                self.spare = Some(frame);
            }
        }
    }
}

pub(super) struct CaptureWorker {
    frames: Consumer<Vec<u8>>,
    recycled: Producer<Vec<u8>>,
    control: CaptureControl,
    signal: CaptureSignalReceiver,
    thread: Option<thread::JoinHandle<()>>,
}

impl CaptureWorker {
    pub(super) fn start(mut source: PcmSource, packet_ms: u16) -> Result<Self> {
        let packet_bytes = source.packet_bytes(packet_ms);
        let (frames_tx, frames_rx) = RingBuffer::new(CAPTURE_QUEUE_ITEMS);
        let (mut recycled_tx, recycled_rx) = RingBuffer::new(CAPTURE_POOL_PACKETS);
        for _ in 0..CAPTURE_POOL_PACKETS {
            recycled_tx
                .push(vec![0; packet_bytes])
                .map_err(|_| anyhow!("failed to initialize capture buffer pool"))?;
        }
        let (signal_tx, signal_rx) = capture_signal_pair()?;
        let control = CaptureControl {
            status: Arc::new(CaptureStatus::new()),
            signal: signal_tx,
        };
        let producer = CaptureProducer::new(frames_tx, recycled_rx, packet_bytes, control.clone());
        let thread_control = control.clone();
        let capture_thread = thread::Builder::new()
            .name("lanpulse-capture".to_string())
            .spawn(move || {
                let error = source
                    .run(producer, packet_ms)
                    .err()
                    .map(|error| format!("{error:#}"));
                thread_control.finish(error);
            })
            .context("failed to start audio capture thread")?;
        Ok(Self {
            frames: frames_rx,
            recycled: recycled_tx,
            control,
            signal: signal_rx,
            thread: Some(capture_thread),
        })
    }

    pub(super) async fn receive(&mut self) -> Result<(u64, Vec<u8>)> {
        loop {
            if let Ok(frame) = self.frames.pop() {
                let dropped = self.control.status.dropped.swap(0, Ordering::Relaxed);
                return Ok((dropped, frame));
            }
            if self
                .control
                .status
                .producer_finished
                .load(Ordering::Acquire)
            {
                return Err(anyhow!(
                    self.control
                        .error_message()
                        .unwrap_or_else(|| "audio capture stopped".to_string())
                ));
            }
            self.signal.wait().await?;
        }
    }

    pub(super) fn recycle(&mut self, frame: Vec<u8>) {
        let _ = self.recycled.push(frame);
    }

    pub(super) async fn shutdown(mut self) {
        self.control
            .status
            .stop_requested
            .store(true, Ordering::Release);
        if let Some(handle) = self.thread.take() {
            let _ = tokio::task::spawn_blocking(move || handle.join()).await;
        }
    }
}

impl Drop for CaptureWorker {
    fn drop(&mut self) {
        self.control
            .status
            .stop_requested
            .store(true, Ordering::Release);
    }
}

const CAPTURE_QUEUE_ITEMS: usize = 4;
const CAPTURE_POOL_PACKETS: usize = CAPTURE_QUEUE_ITEMS + 2;

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicU64, Ordering},
        },
        time::Duration,
    };

    use rtrb::RingBuffer;
    use tokio::time::timeout;

    use super::{CaptureControl, CaptureProducer, CaptureStatus, CaptureWorker};
    use crate::{
        config::AudioConfig,
        media::{source::PcmSource, tone::ToneSource},
    };

    fn control() -> CaptureControl {
        let (signal, _) = super::capture_signal_pair().unwrap();
        CaptureControl {
            status: Arc::new(CaptureStatus {
                stop_requested: AtomicBool::new(false),
                producer_finished: AtomicBool::new(false),
                dropped: AtomicU64::new(0),
                error: Default::default(),
            }),
            signal,
        }
    }

    #[tokio::test]
    async fn producer_coalesces_partial_pcm_into_packet_frames() {
        let (frames_tx, mut frames_rx) = RingBuffer::new(4);
        let (mut recycled_tx, recycled_rx) = RingBuffer::new(4);
        for _ in 0..4 {
            recycled_tx.push(vec![0; 4]).unwrap();
        }
        let mut producer = CaptureProducer::new(frames_tx, recycled_rx, 4, control());

        producer.push_pcm(&[1, 2]);
        assert!(frames_rx.pop().is_err());
        producer.push_pcm(&[3, 4, 5, 6]);
        producer.push_pcm(&[7, 8]);

        assert_eq!(frames_rx.pop().unwrap(), vec![1, 2, 3, 4]);
        assert_eq!(frames_rx.pop().unwrap(), vec![5, 6, 7, 8]);
    }

    #[tokio::test]
    async fn producer_records_drop_when_frame_queue_is_full() {
        let (frames_tx, mut frames_rx) = RingBuffer::new(1);
        let (mut recycled_tx, recycled_rx) = RingBuffer::new(2);
        for _ in 0..2 {
            recycled_tx.push(vec![0; 2]).unwrap();
        }
        let control = control();
        let mut producer = CaptureProducer::new(frames_tx, recycled_rx, 2, control.clone());

        producer.push_pcm(&[1, 2]);
        producer.push_pcm(&[3, 4]);

        assert_eq!(frames_rx.pop().unwrap(), vec![1, 2]);
        assert_eq!(control.status.dropped.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn worker_receives_and_recycles_tone_frames() {
        let audio = AudioConfig {
            sample_rate: 48_000,
            channels: 2,
            sample_format: "s16le".to_string(),
            packet_ms: 5,
            payload_type: 96,
            ssrc: 1,
        };
        let source = PcmSource::Tone(ToneSource::new(audio.sample_rate, audio.channels, 440.0));
        let mut worker = CaptureWorker::start(source, audio.packet_ms).unwrap();

        let (dropped, frame) = timeout(Duration::from_millis(150), worker.receive())
            .await
            .expect("timed out waiting for tone capture")
            .unwrap();
        assert_eq!(dropped, 0);
        assert_eq!(frame.len(), 48_000 / 200 * 2 * 2);
        worker.recycle(frame);
        worker.shutdown().await;
    }
}
