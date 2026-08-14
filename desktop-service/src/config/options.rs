use std::{
    env,
    fs::File,
    io::{Read, Result as IoResult},
    net::SocketAddr,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};

use super::{AudioConfig, AudioSourceMode};

#[derive(Debug, Clone)]
pub struct Options {
    pub host: String,
    pub control_port_start: u16,
    pub control_port_end: u16,
    pub discovery_port: u16,
    pub discovery_port_end: u16,
    pub discovery_enabled: bool,
    pub target: Option<SocketAddr>,
    pub pin: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub packet_ms: u16,
    pub payload_type: u8,
    pub ssrc: u32,
    pub tone_hz: f32,
    pub source: AudioSourceMode,
    pub pipewire_target: Option<String>,
    pub json_events: bool,
}

impl Options {
    pub fn from_env() -> Result<Self> {
        Self::from_args(env::args().skip(1))
    }

    fn from_args(args: impl IntoIterator<Item = String>) -> Result<Self> {
        let mut options = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-h" | "--help" => {
                    Self::print_help();
                    std::process::exit(0);
                }
                "--host" => options.host = next_value(&mut args, "--host")?,
                "--control-port" => {
                    options.control_port_start = parse_value(&mut args, "--control-port")?
                }
                "--control-port-end" => {
                    options.control_port_end = parse_value(&mut args, "--control-port-end")?
                }
                "--discovery-port" => {
                    options.discovery_port = parse_value(&mut args, "--discovery-port")?
                }
                "--discovery-port-end" => {
                    options.discovery_port_end = parse_value(&mut args, "--discovery-port-end")?
                }
                "--no-discovery" => options.discovery_enabled = false,
                "--target" => options.target = Some(parse_value(&mut args, "--target")?),
                "--pin" => options.pin = next_value(&mut args, "--pin")?,
                "--sample-rate" => options.sample_rate = parse_value(&mut args, "--sample-rate")?,
                "--channels" => options.channels = parse_value(&mut args, "--channels")?,
                "--packet-ms" => options.packet_ms = parse_value(&mut args, "--packet-ms")?,
                "--payload-type" => {
                    options.payload_type = parse_value(&mut args, "--payload-type")?
                }
                "--ssrc" => options.ssrc = parse_value(&mut args, "--ssrc")?,
                "--tone-hz" => options.tone_hz = parse_value(&mut args, "--tone-hz")?,
                "--source" => options.source = parse_value(&mut args, "--source")?,
                "--pipewire-target" => {
                    options.pipewire_target = Some(next_value(&mut args, "--pipewire-target")?)
                }
                "--json-events" => options.json_events = true,
                other => return Err(anyhow!("unknown argument: {}", other)),
            }
        }

        options.validate()?;
        Ok(options)
    }

    pub fn audio_config(&self) -> AudioConfig {
        AudioConfig {
            sample_rate: self.sample_rate,
            channels: self.channels,
            sample_format: "s16le".to_string(),
            packet_ms: self.packet_ms,
            payload_type: self.payload_type,
            ssrc: self.ssrc,
        }
    }

    fn validate(&mut self) -> Result<()> {
        if self.control_port_end < self.control_port_start {
            self.control_port_end = self.control_port_start;
        }

        if self.discovery_port_end < self.discovery_port {
            self.discovery_port_end = self.discovery_port;
        }

        if !(4..=12).contains(&self.pin.len())
            || !self.pin.chars().all(|ch| ch.is_ascii_alphanumeric())
        {
            return Err(anyhow!("--pin must be 4-12 ASCII letters or digits"));
        }

        if !(8_000..=192_000).contains(&self.sample_rate) {
            return Err(anyhow!("--sample-rate must be between 8000 and 192000"));
        }

        if !(1..=2).contains(&self.channels) {
            return Err(anyhow!("--channels must be 1 or 2"));
        }

        if !matches!(self.packet_ms, 5 | 10 | 20) {
            return Err(anyhow!("--packet-ms must be one of 5, 10, 20"));
        }

        if !(96..=127).contains(&self.payload_type) {
            return Err(anyhow!(
                "--payload-type must be in dynamic RTP range 96..=127"
            ));
        }

        if !self.tone_hz.is_finite()
            || self.tone_hz <= 0.0
            || self.tone_hz >= self.sample_rate as f32 / 2.0
        {
            return Err(anyhow!(
                "--tone-hz must be finite, greater than 0, and below the Nyquist frequency"
            ));
        }

        Ok(())
    }

    pub fn print_help() {
        println!(
            r#"LanPulse service

Usage:
  lanpulse-service [options]

Options:
  --host <host>                  Control listen host, default 0.0.0.0
  --control-port <port>          First control port, default 4100
  --control-port-end <port>      Last control port, default 4120
  --discovery-port <port>        First UDP discovery port, default 41000
  --discovery-port-end <port>    Last UDP discovery port, default 41020
  --no-discovery                 Disable UDP discovery responder
  --target <ip:port>             Send RTP directly to a receiver, useful for tests
  --pin <pin>                    Pairing PIN, default random 6 digits
  --sample-rate <hz>             Audio sample rate, default 48000
  --channels <n>                 Audio channels, default 2
  --packet-ms <5|10|20>          RTP packet duration, default 5
  --payload-type <96-127>        RTP dynamic payload type, default 96
  --ssrc <u32>                   RTP SSRC, default random
  --tone-hz <hz>                 P0 synthetic tone frequency, default 440
  --source <mode>                auto, pipewire, screencapturekit, or tone
  --pipewire-target <node>       PipeWire target node, default system output sink
  --json-events                  Print machine-readable JSON startup events
  -h, --help                     Show this help
"#
        );
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            control_port_start: 4100,
            control_port_end: 4120,
            discovery_port: 41_000,
            discovery_port_end: 41_020,
            discovery_enabled: true,
            target: None,
            pin: generate_pin(),
            sample_rate: 48_000,
            channels: 2,
            packet_ms: 5,
            payload_type: 96,
            ssrc: random_u32(),
            tone_hz: 440.0,
            source: AudioSourceMode::Auto,
            pipewire_target: None,
            json_events: false,
        }
    }
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    args.next()
        .ok_or_else(|| anyhow!("{} requires a value", flag))
}

fn parse_value<T>(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let value = next_value(args, flag)?;
    value
        .parse::<T>()
        .map_err(|err| anyhow!("invalid value for {}: {} ({})", flag, value, err))
}

fn generate_pin() -> String {
    let value = (random_u32() % 900_000) + 100_000;
    value.to_string()
}

fn random_u32() -> u32 {
    let mut bytes = [0_u8; 4];
    if read_urandom(&mut bytes).is_err() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let fallback = now ^ ((std::process::id() as u128) << 64);
        bytes.copy_from_slice(&(fallback as u32).to_be_bytes());
    }

    u32::from_be_bytes(bytes)
}

fn read_urandom(bytes: &mut [u8]) -> IoResult<()> {
    let mut file = File::open("/dev/urandom")?;
    file.read_exact(bytes)
}

#[cfg(test)]
mod tests {
    use super::{AudioSourceMode, Options};

    fn args<'a>(values: &'a [&'a str]) -> impl Iterator<Item = String> + 'a {
        values.iter().map(|value| value.to_string())
    }

    #[test]
    fn parses_network_audio_source_and_json_flags() {
        let options = Options::from_args(args(&[
            "--host",
            "127.0.0.1",
            "--control-port",
            "4200",
            "--control-port-end",
            "4210",
            "--discovery-port",
            "4300",
            "--discovery-port-end",
            "4310",
            "--target",
            "127.0.0.1:5504",
            "--pin",
            "AB12",
            "--sample-rate",
            "44100",
            "--channels",
            "1",
            "--packet-ms",
            "10",
            "--payload-type",
            "110",
            "--ssrc",
            "99",
            "--tone-hz",
            "880",
            "--source",
            "tone",
            "--pipewire-target",
            "42",
            "--json-events",
        ]))
        .unwrap();

        assert_eq!(options.host, "127.0.0.1");
        assert_eq!(options.control_port_start, 4200);
        assert_eq!(options.control_port_end, 4210);
        assert_eq!(options.discovery_port, 4300);
        assert_eq!(options.discovery_port_end, 4310);
        assert_eq!(options.target.unwrap().to_string(), "127.0.0.1:5504");
        assert_eq!(options.pin, "AB12");
        assert_eq!(options.sample_rate, 44_100);
        assert_eq!(options.channels, 1);
        assert_eq!(options.packet_ms, 10);
        assert_eq!(options.payload_type, 110);
        assert_eq!(options.ssrc, 99);
        assert_eq!(options.tone_hz, 880.0);
        assert_eq!(options.source, AudioSourceMode::Tone);
        assert_eq!(options.pipewire_target.as_deref(), Some("42"));
        assert!(options.json_events);
    }

    #[test]
    fn disables_discovery_when_requested() {
        let options = Options::from_args(args(&["--pin", "1234", "--no-discovery"])).unwrap();

        assert!(!options.discovery_enabled);
    }

    #[test]
    fn clamps_reversed_port_ranges() {
        let options = Options::from_args(args(&[
            "--pin",
            "1234",
            "--control-port",
            "4200",
            "--control-port-end",
            "4100",
            "--discovery-port",
            "4300",
            "--discovery-port-end",
            "4200",
        ]))
        .unwrap();

        assert_eq!(options.control_port_end, 4200);
        assert_eq!(options.discovery_port_end, 4300);
    }

    #[test]
    fn rejects_invalid_audio_options() {
        assert!(Options::from_args(args(&["--pin", "abc"])).is_err());
        assert!(Options::from_args(args(&["--pin", "1234", "--channels", "3"])).is_err());
        assert!(Options::from_args(args(&["--pin", "1234", "--packet-ms", "7"])).is_err());
        assert!(Options::from_args(args(&["--pin", "1234", "--payload-type", "95"])).is_err());
        assert!(Options::from_args(args(&["--pin", "1234", "--tone-hz", "24000"])).is_err());
    }

    #[test]
    fn builds_audio_config_from_options() {
        let options =
            Options::from_args(args(&["--pin", "1234", "--packet-ms", "20", "--ssrc", "7"]))
                .unwrap();
        let audio = options.audio_config();

        assert_eq!(audio.sample_rate, 48_000);
        assert_eq!(audio.channels, 2);
        assert_eq!(audio.sample_format, "s16le");
        assert_eq!(audio.packet_ms, 20);
        assert_eq!(audio.payload_type, 96);
        assert_eq!(audio.ssrc, 7);
    }
}
