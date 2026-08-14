use std::sync::Arc;

use anyhow::Result;
#[cfg(target_os = "linux")]
use anyhow::anyhow;

#[cfg(target_os = "linux")]
use std::os::fd::OwnedFd;
#[cfg(target_os = "linux")]
use tokio::io::unix::AsyncFd;
#[cfg(not(target_os = "linux"))]
use tokio::sync::Notify;

#[cfg(target_os = "linux")]
#[derive(Clone)]
pub(super) struct CaptureSignalSender {
    fd: Arc<OwnedFd>,
}

#[cfg(target_os = "linux")]
pub(super) struct CaptureSignalReceiver {
    fd: AsyncFd<OwnedFd>,
}

#[cfg(target_os = "linux")]
pub(super) fn capture_signal_pair() -> Result<(CaptureSignalSender, CaptureSignalReceiver)> {
    let read_fd = rustix::event::eventfd(
        0,
        rustix::event::EventfdFlags::CLOEXEC | rustix::event::EventfdFlags::NONBLOCK,
    )?;
    let write_fd = rustix::io::dup(&read_fd)?;
    Ok((
        CaptureSignalSender {
            fd: Arc::new(write_fd),
        },
        CaptureSignalReceiver {
            fd: AsyncFd::new(read_fd)?,
        },
    ))
}

#[cfg(target_os = "linux")]
impl CaptureSignalSender {
    pub(super) fn notify(&self) {
        let _ = rustix::io::write(self.fd.as_ref(), &1_u64.to_ne_bytes());
    }
}

#[cfg(target_os = "linux")]
impl CaptureSignalReceiver {
    pub(super) async fn wait(&self) -> Result<()> {
        loop {
            let mut ready = self.fd.readable().await?;
            let mut value = [0_u8; 8];
            match rustix::io::read(self.fd.get_ref(), &mut value[..]) {
                Ok(_) => {
                    ready.clear_ready();
                    return Ok(());
                }
                Err(rustix::io::Errno::AGAIN) => ready.clear_ready(),
                Err(error) => return Err(anyhow!("capture eventfd read failed: {error}")),
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
#[derive(Clone)]
pub(super) struct CaptureSignalSender {
    notify: Arc<Notify>,
}

#[cfg(not(target_os = "linux"))]
pub(super) struct CaptureSignalReceiver {
    notify: Arc<Notify>,
}

#[cfg(not(target_os = "linux"))]
pub(super) fn capture_signal_pair() -> Result<(CaptureSignalSender, CaptureSignalReceiver)> {
    let notify = Arc::new(Notify::new());
    Ok((
        CaptureSignalSender {
            notify: Arc::clone(&notify),
        },
        CaptureSignalReceiver { notify },
    ))
}

#[cfg(not(target_os = "linux"))]
impl CaptureSignalSender {
    pub(super) fn notify(&self) {
        self.notify.notify_one();
    }
}

#[cfg(not(target_os = "linux"))]
impl CaptureSignalReceiver {
    pub(super) async fn wait(&self) -> Result<()> {
        self.notify.notified().await;
        Ok(())
    }
}
