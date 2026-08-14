use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AudioSource {
    Auto,
    PipeWire,
    ScreenCaptureKit,
    Tone,
}

impl AudioSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::PipeWire => "pipewire",
            Self::ScreenCaptureKit => "screencapturekit",
            Self::Tone => "tone",
        }
    }

    pub fn available() -> &'static [Self] {
        #[cfg(target_os = "linux")]
        const SOURCES: &[AudioSource] =
            &[AudioSource::Auto, AudioSource::PipeWire, AudioSource::Tone];
        #[cfg(target_os = "macos")]
        const SOURCES: &[AudioSource] = &[
            AudioSource::Auto,
            AudioSource::ScreenCaptureKit,
            AudioSource::Tone,
        ];
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        const SOURCES: &[AudioSource] = &[AudioSource::Auto, AudioSource::Tone];
        SOURCES
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub start_service_on_launch: bool,
    pub minimize_to_tray: bool,
    pub audio_source: AudioSource,
    pub packet_ms: u16,
    pub control_port_start: u16,
    pub control_port_end: u16,
    pub discovery_port_start: u16,
    pub discovery_port_end: u16,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            start_service_on_launch: true,
            minimize_to_tray: true,
            audio_source: AudioSource::Auto,
            packet_ms: 5,
            control_port_start: 4100,
            control_port_end: 4199,
            discovery_port_start: 41_000,
            discovery_port_end: 41_020,
        }
    }
}

impl AppSettings {
    pub fn sanitize(&mut self) {
        if !matches!(self.packet_ms, 5 | 10 | 20) {
            self.packet_ms = 5;
        }
        self.control_port_start = self.control_port_start.max(1);
        self.control_port_end = self.control_port_end.max(self.control_port_start);
        self.discovery_port_start = self.discovery_port_start.max(1);
        self.discovery_port_end = self.discovery_port_end.max(self.discovery_port_start);
    }

    pub fn service_options_changed(&self, other: &Self) -> bool {
        self.audio_source != other.audio_source
            || self.packet_ms != other.packet_ms
            || self.control_port_start != other.control_port_start
            || self.control_port_end != other.control_port_end
            || self.discovery_port_start != other.discovery_port_start
            || self.discovery_port_end != other.discovery_port_end
    }
}

#[cfg(test)]
mod tests {
    use super::{AppSettings, AudioSource};

    #[test]
    fn audio_sources_format_as_service_cli_values() {
        assert_eq!(AudioSource::Auto.as_str(), "auto");
        assert_eq!(AudioSource::PipeWire.as_str(), "pipewire");
        assert_eq!(AudioSource::ScreenCaptureKit.as_str(), "screencapturekit");
        assert_eq!(AudioSource::Tone.as_str(), "tone");
        assert!(AudioSource::available().contains(&AudioSource::Auto));
        assert!(AudioSource::available().contains(&AudioSource::Tone));
    }

    #[test]
    fn sanitizes_invalid_port_ranges() {
        let mut settings = AppSettings {
            control_port_start: 0,
            control_port_end: 0,
            packet_ms: 7,
            discovery_port_start: 42_000,
            discovery_port_end: 41_000,
            ..AppSettings::default()
        };

        settings.sanitize();

        assert_eq!(settings.control_port_start, 1);
        assert_eq!(settings.control_port_end, 1);
        assert_eq!(settings.packet_ms, 5);
        assert_eq!(settings.discovery_port_start, 42_000);
        assert_eq!(settings.discovery_port_end, 42_000);
    }

    #[test]
    fn service_options_changed_ignores_window_preferences() {
        let old = AppSettings::default();
        let new = AppSettings {
            start_service_on_launch: !old.start_service_on_launch,
            minimize_to_tray: !old.minimize_to_tray,
            ..old.clone()
        };

        assert!(!old.service_options_changed(&new));
    }

    #[test]
    fn service_options_changed_detects_runtime_options() {
        let old = AppSettings::default();

        assert!(old.service_options_changed(&AppSettings {
            packet_ms: 10,
            ..old.clone()
        }));
        assert!(old.service_options_changed(&AppSettings {
            control_port_start: old.control_port_start + 1,
            ..old.clone()
        }));
        assert!(old.service_options_changed(&AppSettings {
            discovery_port_end: old.discovery_port_end + 1,
            ..old.clone()
        }));
    }
}
