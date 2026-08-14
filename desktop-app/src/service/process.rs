use std::{
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::Child,
    sync::mpsc,
    time::Duration,
};

use super::model::{ServiceError, ServiceInfo};

pub(super) fn wait_for_ready(
    stdout: impl Read + Send + 'static,
) -> Result<ServiceInfo, ServiceError> {
    wait_for_ready_with_timeout(stdout, Duration::from_secs(5))
}

fn wait_for_ready_with_timeout(
    stdout: impl Read + Send + 'static,
    timeout: Duration,
) -> Result<ServiceInfo, ServiceError> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(info) = parse_ready_line(trimmed) {
                let _ = tx.send(info);
                break;
            }
        }
    });

    rx.recv_timeout(timeout)
        .map_err(|error| ServiceError::ReadyTimeout(error.to_string()))
}

fn parse_ready_line(line: &str) -> Option<ServiceInfo> {
    serde_json::from_str::<ServiceInfo>(line)
        .ok()
        .filter(ServiceInfo::is_ready_event)
}

pub(super) fn find_service_path() -> Result<PathBuf, ServiceError> {
    for key in ["LANPULSE_SERVICE_PATH", "LANPULSE_DAEMON_PATH"] {
        if let Ok(path) = std::env::var(key) {
            let path = PathBuf::from(path);
            if path.exists() {
                return Ok(path);
            }
        }
    }

    let current_exe = std::env::current_exe().ok();
    let current_dir = std::env::current_dir().ok();
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidates = candidate_service_paths(
        service_exe_name(),
        current_exe.as_deref(),
        &manifest_dir,
        current_dir.as_deref(),
    );

    candidates
        .into_iter()
        .find(|path| path.exists())
        .ok_or(ServiceError::NotFound)
}

fn service_exe_name() -> &'static str {
    if cfg!(windows) {
        "lanpulse-service.exe"
    } else {
        "lanpulse-service"
    }
}

fn candidate_service_paths(
    exe_name: &str,
    current_exe: Option<&Path>,
    manifest_dir: &Path,
    current_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(current_exe) = current_exe
        && let Some(parent) = current_exe.parent()
    {
        candidates.push(parent.join(exe_name));
    }

    candidates.push(manifest_dir.join("../target/debug").join(exe_name));
    candidates.push(manifest_dir.join("../target/release").join(exe_name));

    if let Some(current_dir) = current_dir {
        candidates.push(current_dir.join("../target/debug").join(exe_name));
        candidates.push(current_dir.join("../target/release").join(exe_name));
        candidates.push(current_dir.join("target/debug").join(exe_name));
        candidates.push(current_dir.join("target/release").join(exe_name));
    }

    candidates
}

pub(super) fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use std::{
        io::Cursor,
        path::{Path, PathBuf},
    };

    use super::{
        candidate_service_paths, parse_ready_line, service_exe_name, wait_for_ready,
        wait_for_ready_with_timeout,
    };
    use crate::service::ServiceError;

    fn service_info_json(event: &str) -> String {
        format!(
            r#"{{"event":"{event}","control_url":"http://127.0.0.1:4100","control_port":4100,"discovery_port":null,"pin":"123456","audio":{{"sample_rate":48000,"channels":2,"sample_format":"s16le","packet_ms":5,"payload_type":96,"ssrc":1}},"source":"tone","direct_target":null}}"#
        )
    }

    #[test]
    fn wait_for_ready_ignores_logs_until_json_event() {
        let stdout = Cursor::new(format!("noise\n{}\n", service_info_json("ready")).into_bytes());

        let info = wait_for_ready(stdout).unwrap();

        assert_eq!(info.control_url, "http://127.0.0.1:4100");
        assert_eq!(info.control_port, 4100);
        assert_eq!(info.pin, "123456");
    }

    #[test]
    fn wait_for_ready_times_out_when_stdout_has_no_ready_event() {
        let error = wait_for_ready_with_timeout(
            Cursor::new(b"not json\nalso not json\n".to_vec()),
            std::time::Duration::from_millis(20),
        )
        .unwrap_err();

        assert!(matches!(error, ServiceError::ReadyTimeout(_)));
    }

    #[test]
    fn wait_for_ready_skips_non_ready_service_info_events() {
        let stdout = Cursor::new(
            format!(
                "{}\n{}\n",
                service_info_json("starting"),
                service_info_json("ready")
            )
            .into_bytes(),
        );

        let info = wait_for_ready(stdout).unwrap();

        assert_eq!(info.control_url, "http://127.0.0.1:4100");
        assert!(info.is_ready_event());
    }

    #[test]
    fn parse_ready_line_rejects_invalid_json_and_non_ready_events() {
        assert!(parse_ready_line("not json").is_none());
        assert!(parse_ready_line(&service_info_json("starting")).is_none());
        assert!(parse_ready_line(&service_info_json("ready")).is_some());
    }

    #[test]
    fn service_path_candidates_cover_runtime_and_workspace_locations() {
        let candidates = candidate_service_paths(
            "lanpulse-service",
            Some(Path::new("/app/bin/lanpulse-app")),
            Path::new("/repo/desktop-app"),
            Some(Path::new("/repo")),
        );

        assert_eq!(
            candidates,
            vec![
                PathBuf::from("/app/bin/lanpulse-service"),
                PathBuf::from("/repo/desktop-app/../target/debug/lanpulse-service"),
                PathBuf::from("/repo/desktop-app/../target/release/lanpulse-service"),
                PathBuf::from("/repo/../target/debug/lanpulse-service"),
                PathBuf::from("/repo/../target/release/lanpulse-service"),
                PathBuf::from("/repo/target/debug/lanpulse-service"),
                PathBuf::from("/repo/target/release/lanpulse-service"),
            ]
        );
    }

    #[test]
    fn service_exe_name_matches_current_platform() {
        if cfg!(windows) {
            assert_eq!(service_exe_name(), "lanpulse-service.exe");
        } else {
            assert_eq!(service_exe_name(), "lanpulse-service");
        }
    }
}
