use std::f32::consts::TAU;

use super::packet::frames_per_packet;

pub struct ToneSource {
    sample_rate: u32,
    channels: u16,
    phase: f32,
    phase_step: f32,
}

impl ToneSource {
    pub fn new(sample_rate: u32, channels: u16, hz: f32) -> Self {
        Self {
            sample_rate,
            channels,
            phase: 0.0,
            phase_step: hz * TAU / sample_rate as f32,
        }
    }

    pub fn next_packet(&mut self, packet_ms: u16) -> Vec<u8> {
        let frames = frames_per_packet(self.sample_rate, packet_ms);
        let mut payload = vec![0; frames as usize * self.channels as usize * 2];
        self.fill_packet(&mut payload);
        payload
    }

    pub(super) fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub(super) fn channels(&self) -> u16 {
        self.channels
    }

    pub(super) fn fill_packet(&mut self, payload: &mut [u8]) {
        let frame_bytes = usize::from(self.channels) * 2;

        for frame in payload.chunks_exact_mut(frame_bytes) {
            let sample = (self.phase.sin() * 0.20 * i16::MAX as f32) as i16;
            self.phase += self.phase_step;
            if self.phase >= TAU {
                self.phase -= TAU;
            }

            for channel in frame.chunks_exact_mut(2) {
                channel.copy_from_slice(&sample.to_le_bytes());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ToneSource;

    #[test]
    fn tone_payload_matches_pcm_shape() {
        let mut tone = ToneSource::new(48_000, 2, 440.0);
        let payload = tone.next_packet(10);

        assert_eq!(payload.len(), 480 * 2 * 2);
    }

    #[test]
    fn duplicates_each_sample_across_channels() {
        let mut tone = ToneSource::new(48_000, 2, 440.0);
        let mut payload = vec![0; 16];
        tone.fill_packet(&mut payload);

        for frame in payload.chunks_exact(4) {
            assert_eq!(&frame[0..2], &frame[2..4]);
        }
    }
}
