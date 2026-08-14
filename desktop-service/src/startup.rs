use std::io::{self, Write};

use anyhow::Result;
use serde::Serialize;

use crate::config::{AudioConfig, Options};

#[derive(Debug, Serialize)]
pub struct StartupEvent {
    event: &'static str,
    control_url: String,
    control_port: u16,
    discovery_port: Option<u16>,
    pin: String,
    audio: AudioConfig,
    source: String,
    direct_target: Option<String>,
}

pub fn print_ready(
    options: &Options,
    control_url: &str,
    control_port: u16,
    discovery_port: Option<u16>,
) -> Result<()> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    write_ready(
        &mut stdout,
        options,
        control_url,
        control_port,
        discovery_port,
    )
}

fn write_ready(
    writer: &mut impl Write,
    options: &Options,
    control_url: &str,
    control_port: u16,
    discovery_port: Option<u16>,
) -> Result<()> {
    for line in ready_lines(options, control_url, control_port, discovery_port)? {
        writeln!(writer, "{line}")?;
    }
    Ok(())
}

fn ready_lines(
    options: &Options,
    control_url: &str,
    control_port: u16,
    discovery_port: Option<u16>,
) -> Result<Vec<String>> {
    let mut lines = vec![String::new()];
    if options.json_events {
        lines.push(serde_json::to_string(&startup_event(
            options,
            control_url.to_string(),
            control_port,
            discovery_port,
        ))?);
    } else {
        lines.push("LanPulse service running".to_string());
        lines.push(format!("Control URL: {control_url}"));
        if let Some(discovery_port) = discovery_port {
            lines.push(format!("Discovery: UDP {discovery_port}"));
        } else {
            lines.push("Discovery: disabled".to_string());
        }
        lines.push(format!("PIN: {}", options.pin));
        lines.push(format!("Source: {}", options.source.as_str()));
        lines.push(format!(
            "Audio: {}Hz, {}ch, {}ms packet, RTP payload={}, ssrc={}",
            options.sample_rate,
            options.channels,
            options.packet_ms,
            options.payload_type,
            options.ssrc
        ));
        if let Some(target) = options.target {
            lines.push(format!("Direct RTP target: {target}"));
        } else {
            lines.push("Waiting for mobile /api/connect with UDP port".to_string());
        }
        lines.push("Stop: Ctrl+C".to_string());
    }
    lines.push(String::new());
    Ok(lines)
}

fn startup_event(
    options: &Options,
    control_url: String,
    control_port: u16,
    discovery_port: Option<u16>,
) -> StartupEvent {
    StartupEvent {
        event: "ready",
        control_url,
        control_port,
        discovery_port,
        pin: options.pin.clone(),
        audio: options.audio_config(),
        source: options.source.as_str().to_string(),
        direct_target: options.target.map(|target| target.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ready_lines, startup_event, write_ready};
    use crate::config::Options;

    #[test]
    fn serializes_ready_event_with_audio_and_direct_target() {
        let mut options = Options::default();
        options.pin = "123456".to_string();
        options.ssrc = 42;
        options.target = Some("127.0.0.1:5504".parse().unwrap());

        let event = startup_event(
            &options,
            "http://127.0.0.1:4100".to_string(),
            4100,
            Some(41000),
        );
        let value = serde_json::to_value(event).unwrap();

        assert_eq!(value["event"], "ready");
        assert_eq!(value["control_url"], "http://127.0.0.1:4100");
        assert_eq!(value["discovery_port"], 41000);
        assert_eq!(value["pin"], "123456");
        assert_eq!(value["direct_target"], "127.0.0.1:5504");
        assert_eq!(
            value["audio"],
            json!({
                "sample_rate": 48000,
                "channels": 2,
                "sample_format": "s16le",
                "packet_ms": 5,
                "payload_type": 96,
                "ssrc": 42
            })
        );
    }

    #[test]
    fn human_ready_lines_show_disabled_discovery_and_waiting_mode() {
        let mut options = Options::default();
        options.pin = "ABCD12".to_string();
        options.ssrc = 7;

        let lines = ready_lines(&options, "http://192.168.1.10:4100", 4100, None).unwrap();

        assert_eq!(lines.first().map(String::as_str), Some(""));
        assert!(lines.contains(&"LanPulse service running".to_string()));
        assert!(lines.contains(&"Control URL: http://192.168.1.10:4100".to_string()));
        assert!(lines.contains(&"Discovery: disabled".to_string()));
        assert!(lines.contains(&"PIN: ABCD12".to_string()));
        assert!(lines.contains(&"Source: auto".to_string()));
        assert!(
            lines.contains(&"Audio: 48000Hz, 2ch, 5ms packet, RTP payload=96, ssrc=7".to_string())
        );
        assert!(lines.contains(&"Waiting for mobile /api/connect with UDP port".to_string()));
        assert_eq!(lines.last().map(String::as_str), Some(""));
    }

    #[test]
    fn json_ready_lines_encode_null_optional_fields() {
        let mut options = Options::default();
        options.json_events = true;
        options.pin = "9999".to_string();

        let lines = ready_lines(&options, "http://127.0.0.1:4100", 4100, None).unwrap();
        let value: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();

        assert_eq!(lines.len(), 3);
        assert_eq!(value["event"], "ready");
        assert_eq!(value["discovery_port"], serde_json::Value::Null);
        assert_eq!(value["direct_target"], serde_json::Value::Null);
    }

    #[test]
    fn write_ready_keeps_blank_line_framing() {
        let mut output = Vec::new();
        let mut options = Options::default();
        options.pin = "1234".to_string();

        write_ready(
            &mut output,
            &options,
            "http://127.0.0.1:4100",
            4100,
            Some(41000),
        )
        .unwrap();
        let output = String::from_utf8(output).unwrap();

        assert!(output.starts_with("\nLanPulse service running\n"));
        assert!(output.ends_with("Stop: Ctrl+C\n\n"));
    }
}
