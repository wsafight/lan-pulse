use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub sample_format: String,
    pub packet_ms: u16,
    pub payload_type: u8,
    pub ssrc: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AudioSourceMode {
    Auto,
    PipeWire,
    ScreenCaptureKit,
    Tone,
}

impl AudioSourceMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::PipeWire => "pipewire",
            Self::ScreenCaptureKit => "screencapturekit",
            Self::Tone => "tone",
        }
    }
}

impl std::str::FromStr for AudioSourceMode {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "auto" => Ok(Self::Auto),
            "pipewire" => Ok(Self::PipeWire),
            "screencapturekit" | "screen-capture-kit" => Ok(Self::ScreenCaptureKit),
            "tone" => Ok(Self::Tone),
            other => Err(anyhow!(
                "invalid audio source: {}; expected auto, pipewire, screencapturekit, or tone",
                other
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AudioSourceMode;

    #[test]
    fn parses_screen_capture_kit_aliases() {
        assert_eq!(
            "screencapturekit".parse::<AudioSourceMode>().unwrap(),
            AudioSourceMode::ScreenCaptureKit
        );
        assert_eq!(
            "screen-capture-kit".parse::<AudioSourceMode>().unwrap(),
            AudioSourceMode::ScreenCaptureKit
        );
    }

    #[test]
    fn rejects_unknown_audio_source() {
        assert!("alsa".parse::<AudioSourceMode>().is_err());
    }

    #[test]
    fn audio_source_modes_format_as_cli_values() {
        assert_eq!(AudioSourceMode::Auto.as_str(), "auto");
        assert_eq!(AudioSourceMode::PipeWire.as_str(), "pipewire");
        assert_eq!(
            AudioSourceMode::ScreenCaptureKit.as_str(),
            "screencapturekit"
        );
        assert_eq!(AudioSourceMode::Tone.as_str(), "tone");
    }
}
