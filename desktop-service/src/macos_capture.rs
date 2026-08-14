use std::{
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use screencapturekit::prelude::*;

use crate::{
    config::AudioConfig,
    media::{CaptureControl, CaptureProducer},
};

pub(crate) fn run(
    audio: AudioConfig,
    producer: CaptureProducer,
    control: CaptureControl,
) -> Result<()> {
    validate_audio_config(&audio)?;

    let content = SCShareableContent::get()
        .context("unable to read shareable displays; allow Screen Recording in System Settings")?;
    let display = content
        .displays()
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("ScreenCaptureKit found no display to capture"))?;
    let filter = SCContentFilter::create()
        .with_display(&display)
        .with_excluding_windows(&[])
        .build();
    let configuration = SCStreamConfiguration::new()
        .with_width(2)
        .with_height(2)
        .with_captures_audio(true)
        .with_sample_rate(audio.sample_rate as i32)
        .with_channel_count(i32::from(audio.channels))
        .with_excludes_current_process_audio(true);

    let producer = Arc::new(Mutex::new(CaptureHandler {
        producer,
        converted: Vec::new(),
    }));
    let stream_error = Arc::new(Mutex::new(None::<String>));
    let delegate_error = Arc::clone(&stream_error);
    let mut stream = SCStream::new_with_delegate(
        &filter,
        &configuration,
        ErrorHandler::new(move |error| {
            let mut current = delegate_error
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if current.is_none() {
                *current = Some(error.to_string());
            }
        }),
    );

    let callback_producer = Arc::clone(&producer);
    let channels = usize::from(audio.channels);
    let handler = stream.add_output_handler(
        move |sample: CMSampleBuffer, output_type: SCStreamOutputType| {
            if output_type != SCStreamOutputType::Audio {
                return;
            }
            let Some(buffers) = sample.audio_buffer_list() else {
                return;
            };
            let mut handler = callback_producer
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            handler.push_float_pcm(&buffers, channels);
        },
        SCStreamOutputType::Audio,
    );
    if handler.is_none() {
        return Err(anyhow!(
            "ScreenCaptureKit rejected the audio output handler"
        ));
    }

    stream
        .start_capture()
        .context("unable to start ScreenCaptureKit system audio capture")?;
    while !control.stop_requested() {
        if let Some(error) = control.error_message() {
            let _ = stream.stop_capture();
            return Err(anyhow!(error));
        }
        if let Some(error) = stream_error
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        {
            let _ = stream.stop_capture();
            return Err(anyhow!("ScreenCaptureKit stopped: {error}"));
        }
        thread::sleep(Duration::from_millis(20));
    }
    stream
        .stop_capture()
        .context("unable to stop ScreenCaptureKit capture")?;
    Ok(())
}

fn validate_audio_config(audio: &AudioConfig) -> Result<()> {
    if !matches!(audio.sample_rate, 8_000 | 16_000 | 24_000 | 48_000) {
        return Err(anyhow!(
            "ScreenCaptureKit does not support {} Hz capture",
            audio.sample_rate
        ));
    }
    Ok(())
}

struct CaptureHandler {
    producer: CaptureProducer,
    converted: Vec<u8>,
}

impl CaptureHandler {
    fn push_float_pcm(&mut self, buffers: &screencapturekit::cm::AudioBufferList, channels: usize) {
        if buffers.num_buffers() == 1 {
            if let Some(buffer) = buffers.get(0) {
                let sample_count = buffer.data().len() / 4;
                self.converted.resize(sample_count * 2, 0);
                for (input, output) in buffer
                    .data()
                    .chunks_exact(4)
                    .zip(self.converted.chunks_exact_mut(2))
                {
                    let sample = f32::from_ne_bytes(input.try_into().expect("four-byte float"));
                    output.copy_from_slice(&float_to_s16(sample).to_le_bytes());
                }
                self.producer.push_pcm(&self.converted);
            }
            return;
        }

        if buffers.num_buffers() < channels {
            self.producer.fail(format!(
                "ScreenCaptureKit returned {} audio buffers for {channels} channels",
                buffers.num_buffers()
            ));
            return;
        }
        let frames = (0..channels)
            .filter_map(|channel| buffers.get(channel))
            .map(|buffer| buffer.data().len() / 4)
            .min()
            .unwrap_or(0);
        self.converted.resize(frames * channels * 2, 0);
        for frame in 0..frames {
            for channel in 0..channels {
                let data = buffers
                    .get(channel)
                    .expect("validated ScreenCaptureKit channel")
                    .data();
                let offset = frame * 4;
                let sample = f32::from_ne_bytes(
                    data[offset..offset + 4]
                        .try_into()
                        .expect("four-byte float"),
                );
                let output_offset = (frame * channels + channel) * 2;
                self.converted[output_offset..output_offset + 2]
                    .copy_from_slice(&float_to_s16(sample).to_le_bytes());
            }
        }
        self.producer.push_pcm(&self.converted);
    }
}

fn float_to_s16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}

#[cfg(test)]
mod tests {
    use crate::config::AudioConfig;

    use super::{float_to_s16, validate_audio_config};

    #[test]
    fn converts_float_pcm_to_s16_with_clipping() {
        assert_eq!(float_to_s16(0.0), 0);
        assert_eq!(float_to_s16(1.0), i16::MAX);
        assert_eq!(float_to_s16(-1.0), -i16::MAX);
        assert_eq!(float_to_s16(2.0), i16::MAX);
    }

    #[test]
    fn validates_screen_capture_kit_sample_rates_before_starting_capture() {
        let mut audio = AudioConfig {
            sample_rate: 48_000,
            channels: 2,
            sample_format: "s16le".to_string(),
            packet_ms: 5,
            payload_type: 96,
            ssrc: 1,
        };

        assert!(validate_audio_config(&audio).is_ok());
        audio.sample_rate = 44_100;

        let error = validate_audio_config(&audio).unwrap_err().to_string();
        assert!(error.contains("44100 Hz"));
    }
}
