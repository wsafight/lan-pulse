use std::{io::Cursor, time::Duration};

use anyhow::{Context, Result, anyhow};
use pipewire::{
    self as pw,
    properties::properties,
    spa::{self, pod::Pod},
};

use crate::{
    config::AudioConfig,
    media::{CaptureControl, CaptureProducer},
};

struct PipeWireData {
    producer: CaptureProducer,
    format: spa::param::audio::AudioInfoRaw,
}

pub(crate) fn run(
    audio: AudioConfig,
    target: Option<String>,
    producer: CaptureProducer,
    control: CaptureControl,
) -> Result<()> {
    pw::init();

    let mainloop = pw::main_loop::MainLoopRc::new(None).context("create PipeWire main loop")?;
    let context =
        pw::context::ContextRc::new(&mainloop, None).context("create PipeWire context")?;
    let core = context.connect_rc(None).context("connect to PipeWire")?;
    let packet_frames = audio.sample_rate.saturating_mul(audio.packet_ms as u32) / 1_000;

    let mut props = properties! {
        *pw::keys::APP_NAME => "LanPulse",
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::STREAM_CAPTURE_SINK => "true",
    };
    props.insert(
        *pw::keys::NODE_LATENCY,
        format!("{packet_frames}/{}", audio.sample_rate),
    );
    props.insert(*pw::keys::NODE_RATE, format!("1/{}", audio.sample_rate));
    if let Some(target) = target {
        props.insert(*pw::keys::TARGET_OBJECT, target);
    }

    let stream = pw::stream::StreamBox::new(&core, "lanpulse-capture", props)
        .context("create PipeWire capture stream")?;
    let data = PipeWireData {
        producer,
        format: Default::default(),
    };

    let state_loop = mainloop.clone();
    let format_loop = mainloop.clone();
    let expected_rate = audio.sample_rate;
    let expected_channels = u32::from(audio.channels);
    let _listener = stream
        .add_local_listener_with_user_data(data)
        .state_changed(move |_, data, _, new| {
            if let pw::stream::StreamState::Error(error) = new {
                data.producer
                    .fail(format!("PipeWire stream error: {error}"));
                state_loop.quit();
            }
        })
        .param_changed(move |_, data, id, param| {
            let Some(param) = param else {
                return;
            };
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            let parsed = data.format.parse(param);
            if parsed.is_err()
                || data.format.format() != spa::param::audio::AudioFormat::S16LE
                || data.format.rate() != expected_rate
                || data.format.channels() != expected_channels
            {
                data.producer
                    .fail("PipeWire negotiated an incompatible audio format".to_string());
                format_loop.quit();
            }
        })
        .process(|stream, data| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let Some(plane) = buffer.datas_mut().first_mut() else {
                return;
            };
            let offset = plane.chunk().offset() as usize;
            let size = plane.chunk().size() as usize;
            let Some(bytes) = plane.data() else {
                return;
            };
            let Some(end) = offset.checked_add(size) else {
                return;
            };
            if let Some(pcm) = bytes.get(offset..end) {
                data.producer.push_pcm(pcm);
            }
        })
        .register()
        .context("register PipeWire stream listener")?;

    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::S16LE);
    audio_info.set_rate(audio.sample_rate);
    audio_info.set_channels(u32::from(audio.channels));
    let object = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    let values = spa::pod::serialize::PodSerializer::serialize(
        Cursor::new(Vec::new()),
        &spa::pod::Value::Object(object),
    )
    .context("serialize PipeWire audio format")?
    .0
    .into_inner();
    let mut params =
        [Pod::from_bytes(&values).ok_or_else(|| anyhow!("invalid PipeWire format pod"))?];

    stream
        .connect(
            spa::utils::Direction::Input,
            None,
            pw::stream::StreamFlags::AUTOCONNECT
                | pw::stream::StreamFlags::MAP_BUFFERS
                | pw::stream::StreamFlags::RT_PROCESS,
            &mut params,
        )
        .context("connect PipeWire capture stream")?;

    let stop_loop = mainloop.clone();
    let stop_control = control.clone();
    let stop_timer = mainloop.loop_().add_timer(move |_| {
        if stop_control.stop_requested() {
            stop_loop.quit();
        }
    });
    stop_timer
        .update_timer(
            Some(Duration::from_millis(STOP_POLL_MS)),
            Some(Duration::from_millis(STOP_POLL_MS)),
        )
        .into_result()
        .context("arm PipeWire stop timer")?;

    mainloop.run();
    if control.stop_requested() {
        Ok(())
    } else {
        Err(anyhow!(control.error_message().unwrap_or_else(|| {
            "PipeWire capture stopped unexpectedly".to_string()
        })))
    }
}

const STOP_POLL_MS: u64 = 20;
