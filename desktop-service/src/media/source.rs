use std::{thread, time::Duration, time::Instant};

use anyhow::{Result, anyhow};

use crate::config::{AudioConfig, AudioSourceMode};

use super::{
    capture::CaptureProducer,
    packet::{frames_per_packet, packet_bytes},
    tone::ToneSource,
};

pub(super) enum PcmSource {
    #[cfg(target_os = "linux")]
    PipeWire {
        audio: AudioConfig,
        target: Option<String>,
    },
    #[cfg(target_os = "macos")]
    ScreenCaptureKit {
        audio: AudioConfig,
    },
    Tone(ToneSource),
}

impl PcmSource {
    pub(super) fn new(
        mode: AudioSourceMode,
        audio: &AudioConfig,
        tone_hz: f32,
        _pipewire_target: Option<&str>,
    ) -> Result<Self> {
        match mode {
            AudioSourceMode::Tone => Ok(Self::Tone(ToneSource::new(
                audio.sample_rate,
                audio.channels,
                tone_hz,
            ))),
            AudioSourceMode::PipeWire => {
                #[cfg(target_os = "linux")]
                {
                    return Ok(Self::PipeWire {
                        audio: audio.clone(),
                        target: _pipewire_target.map(ToOwned::to_owned),
                    });
                }
                #[cfg(not(target_os = "linux"))]
                Err(anyhow!("PipeWire capture is only available on Linux"))
            }
            AudioSourceMode::ScreenCaptureKit => {
                #[cfg(target_os = "macos")]
                {
                    Ok(Self::ScreenCaptureKit {
                        audio: audio.clone(),
                    })
                }
                #[cfg(not(target_os = "macos"))]
                Err(anyhow!(
                    "ScreenCaptureKit capture is only available on macOS"
                ))
            }
            AudioSourceMode::Auto => {
                #[cfg(target_os = "linux")]
                {
                    return Ok(Self::PipeWire {
                        audio: audio.clone(),
                        target: _pipewire_target.map(ToOwned::to_owned),
                    });
                }
                #[cfg(target_os = "macos")]
                {
                    Ok(Self::ScreenCaptureKit {
                        audio: audio.clone(),
                    })
                }
                #[cfg(not(any(target_os = "linux", target_os = "macos")))]
                Err(anyhow!(
                    "automatic system audio capture is not available on this platform"
                ))
            }
        }
    }

    pub(super) fn label(&self) -> &'static str {
        match self {
            #[cfg(target_os = "linux")]
            Self::PipeWire { .. } => "pipewire",
            #[cfg(target_os = "macos")]
            Self::ScreenCaptureKit { .. } => "screencapturekit",
            Self::Tone(_) => "tone",
        }
    }

    pub(super) fn packet_bytes(&self, packet_ms: u16) -> usize {
        match self {
            #[cfg(target_os = "linux")]
            Self::PipeWire { audio, .. } => packet_bytes(audio),
            #[cfg(target_os = "macos")]
            Self::ScreenCaptureKit { audio } => packet_bytes(audio),
            Self::Tone(source) => {
                frames_per_packet(source.sample_rate(), packet_ms) as usize
                    * usize::from(source.channels())
                    * 2
            }
        }
    }

    pub(super) fn run(&mut self, mut producer: CaptureProducer, packet_ms: u16) -> Result<()> {
        match self {
            Self::Tone(source) => {
                let packet_duration = Duration::from_millis(u64::from(packet_ms));
                let mut next_packet_at = Instant::now();
                while !producer.control.stop_requested() {
                    next_packet_at += packet_duration;
                    let now = Instant::now();
                    if next_packet_at > now {
                        thread::sleep(next_packet_at - now);
                    } else if now.duration_since(next_packet_at) > packet_duration {
                        next_packet_at = now;
                    }

                    let Some(mut frame) = producer.acquire_frame() else {
                        producer.control.record_dropped(1);
                        continue;
                    };
                    source.fill_packet(&mut frame);
                    producer.submit_frame(frame);
                }
                Ok(())
            }
            #[cfg(target_os = "linux")]
            Self::PipeWire { audio, target } => {
                let control = producer.control.clone();
                crate::pipewire_capture::run(audio.clone(), target.clone(), producer, control)
            }
            #[cfg(target_os = "macos")]
            Self::ScreenCaptureKit { audio } => {
                let control = producer.control.clone();
                crate::macos_capture::run(audio.clone(), producer, control)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::{AudioConfig, AudioSourceMode};

    use super::PcmSource;

    fn audio() -> AudioConfig {
        AudioConfig {
            sample_rate: 48_000,
            channels: 2,
            sample_format: "s16le".to_string(),
            packet_ms: 5,
            payload_type: 96,
            ssrc: 1,
        }
    }

    #[test]
    fn creates_tone_source_on_every_platform() {
        let source = PcmSource::new(AudioSourceMode::Tone, &audio(), 440.0, None).unwrap();

        assert_eq!(source.label(), "tone");
        assert_eq!(source.packet_bytes(5), 960);
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn rejects_pipewire_on_non_linux_platforms() {
        assert!(PcmSource::new(AudioSourceMode::PipeWire, &audio(), 440.0, None).is_err());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn rejects_screen_capture_kit_on_non_macos_platforms() {
        assert!(PcmSource::new(AudioSourceMode::ScreenCaptureKit, &audio(), 440.0, None).is_err());
    }
}
