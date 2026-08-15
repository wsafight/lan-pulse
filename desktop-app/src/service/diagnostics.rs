use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

#[cfg(not(test))]
use std::path::PathBuf;

use chrono::Local;

use super::model::LogEvent;

const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;
#[cfg(not(test))]
const CURRENT_LOG: &str = "lanpulse-desktop.log";
const PREVIOUS_LOG: &str = "lanpulse-desktop.previous.log";

#[cfg(not(test))]
pub(super) fn append(event: &LogEvent) {
    let Some(path) = log_path() else {
        return;
    };
    if let Err(error) = append_to_path(&path, event) {
        eprintln!(
            "unable to write desktop diagnostics to {}: {error}",
            path.display()
        );
    }
}

#[cfg(test)]
pub(super) fn append(_event: &LogEvent) {}

#[cfg(not(test))]
pub(super) fn log_path() -> Option<PathBuf> {
    state_directory().map(|directory| directory.join("lanpulse").join(CURRENT_LOG))
}

#[cfg(not(test))]
fn state_directory() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".local/state"))
        })
}

fn append_to_path(path: &Path, event: &LogEvent) -> std::io::Result<()> {
    let Some(directory) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(directory)?;
    if path
        .metadata()
        .is_ok_and(|metadata| metadata.len() >= MAX_LOG_BYTES)
    {
        let previous = directory.join(PREVIOUS_LOG);
        if previous.exists() {
            fs::remove_file(&previous)?;
        }
        fs::rename(path, previous)?;
    }

    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(
        file,
        "{} event={event:?}",
        Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z")
    )
}

#[cfg(test)]
mod tests {
    use std::{fs, time::SystemTime};

    use super::append_to_path;
    use crate::service::LogEvent;

    #[test]
    fn appends_timestamped_diagnostic_events() {
        let unique = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("lanpulse-diagnostics-{unique}"));
        let path = directory.join("desktop.log");

        append_to_path(&path, &LogEvent::Ready("http://127.0.0.1:4100".to_string())).unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("event=Ready(\"http://127.0.0.1:4100\")"));
        fs::remove_dir_all(directory).unwrap();
    }
}
