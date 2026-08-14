# LanPulse Technical Design

## 1. Summary

The primary architecture for the lowest practical latency when using a phone as a speaker on a local network is:

```text
Rust desktop app + KMP mobile app + platform-native audio + UDP/RTP PCM
```

Web, PCM over WebSocket, WebRTC, and GStreamer are not primary components.

The main reasons are:

- Mobile browsers cannot guarantee background and lock-screen playback.
- WebRTC includes a general-purpose network stack, encryption, congestion control, and a jitter buffer. It is well suited to calling and internet transport, but not to minimizing latency on a LAN.
- GStreamer is mature but heavy, with a larger distribution size and more cross-platform packaging work.
- A local network has enough bandwidth for PCM, so skipping encoding can reduce end-to-end latency.
- A 5 ms PCM/RTP packet fits within a common MTU and avoids the IP fragmentation caused by 10 ms stereo PCM packets.

"Lowest latency" is an engineering choice within this project's constraints, not an absolute claim without measurements. Final acceptance depends on end-to-end acoustic latency, jitter, packet loss, and CPU usage measurements.

## 2. System Architecture

```text
Desktop
  Rust service
    |- platform capture
    |  |- Linux PipeWire system-output capture
    |  |- Windows WASAPI loopback
    |  `- macOS ScreenCaptureKit/CoreAudio
    |- clock / resampler
    |- packetizer
    |- RTP/UDP media sender
    |- pairing server
    `- device/session manager
  Rust native desktop shell
    |- compact egui window
    |- system tray
    `- asynchronous service lifecycle manager

Mobile
  KMP app
    |- commonMain
    |  |- UI state
    |  |- pairing protocol
    |  |- LAN discovery client
    |  `- settings/state machine
    |- androidMain
    |  |- UDP receiver
    |  |- jitter buffer
    |  |- native audio output
    |  |- Foreground Service
    |  `- MediaSession
    `- iosMain
       |- UDP receiver
       |- jitter buffer
       |- native audio output
       |- AVAudioSession
       `- Background Audio
```

## 3. Media Transport

### 3.1 Default Mode: PCM over RTP/UDP

Audio format:

```text
sample_rate: 48000
channels: 2
sample_format: s16le
packet_duration: 5 ms default; 10/20 ms optional
transport: UDP
payload: RTP dynamic payload type
```

Bandwidth:

```text
48000 * 2 channels * 16 bits = 1.536 Mbps
approximately 192 KB/s, excluding UDP/IP/RTP headers
```

LAN Wi-Fi can handle this bandwidth. Compared with Opus, PCM avoids encoding/decoding latency and encoder complexity.

### 3.2 RTP Fields

The protocol follows the RTP semantics in RFC 3550:

- Sequence number for packet-loss and reordering detection.
- Timestamp for the audio sample clock.
- SSRC for media-source identification.
- A dynamic payload type, such as `96`.

Reference: [RFC 3550](https://www.rfc-editor.org/rfc/rfc3550)

### 3.3 Packet Size

Default low-latency mode:

```text
5 ms packet:
240 frames/channel
stereo s16le = 240 * 2 * 2 = 960-byte payload
RTP datagram = 960 + 12 = 972 bytes
```

Compatibility mode:

```text
10 ms packet:
480 frames/channel
stereo s16le = 1920-byte payload
```

With a common IPv4 MTU of 1500 bytes, the maximum UDP payload after the IPv4 and UDP headers is approximately 1472 bytes. A 10 ms PCM/RTP datagram is fragmented at the IP layer. A 5 ms datagram stays below that limit, reduces packetization wait by 5 ms, and avoids the increased loss impact of fragmentation. Therefore, 5 ms is the default. Durations of 10 ms and 20 ms are retained only for compatibility, fewer wakeups, or future encoded modes.

## 4. Playback Buffer

The Android and iOS implementations use native adaptive jitter buffers driven by RTP timestamp and local arrival-time variation. They start conservatively, grow quickly after jitter or an underrun, and shrink only after a sustained stable period to avoid oscillation.

Defaults:

```text
initial_buffer: 15 ms
min_buffer: 10 ms
max_buffer: 60 ms
backlog_reset: 80 ms
```

Strategy:

- Maintain a 10-20 ms buffer under normal network conditions.
- Reorder packets by sequence number.
- Replace lost packets with silence or an attenuated copy of the previous frame.
- Drop old packets when the buffer grows too large, prioritizing real-time playback.
- Correct short-term clock drift by removing or interpolating at most one PCM sample frame per RTP packet.
- Add a lightweight resampler only if long-duration device measurements show that per-frame correction is insufficient or audible.

Current Android behavior:

- Start at 15 ms, shrink to 10 ms on a stable network, and grow toward 60 ms as measured jitter rises.
- Reorder future RTP packets in a bounded, preallocated fixed-slot window.
- Replace a missing packet with silence and expose received/lost counters.
- Track the 32-bit `AudioTrack` playback head across wraparound and keep the queued audio near the adaptive target.
- Treat three seconds without usable audio as a broken stream and reconnect after three seconds.

Current iOS behavior:

- Use the same 15 ms initial and 10-60 ms adaptive targets with an 80 ms bounded reorder backlog.
- Parse RTP in place with a reusable packet pool and convert s16le PCM to native Float32 channel buffers with vDSP.
- Refill a bounded `AVAudioPlayerNode` buffer pool from audio-consumption callbacks instead of a wall-clock timer.
- Recover from AVAudioSession interruptions, route changes, and media-service resets.
- Treat three seconds without audio as a broken stream and retry continuously with network-aware 0.5-5 second backoff.

## 5. Desktop

### 5.1 Rust Service

Responsibilities:

- Pairing.
- Device management.
- Audio capture.
- Audio clock.
- Packetization.
- UDP transmission.
- Status monitoring.

Project structure:

```text
desktop-app/
  Cargo.toml
  src/
    main.rs              # native entry point
    app.rs               # application coordination
    service.rs           # service lifecycle and status polling
    settings.rs          # persisted desktop and service settings
    tray.rs              # system tray
    ui.rs                # egui dashboard and styling
    i18n.rs              # Chinese and English strings

desktop-service/
  Cargo.toml
  src/
    main.rs
    config.rs            # CLI and audio configuration
    discovery.rs         # UDP discovery
    media.rs             # capture boundary and paced sender
    pipewire_capture.rs  # native Linux PipeWire stream
    rtp.rs               # RTP packetizer
    state.rs             # session, device, and counters
```

### 5.2 Linux Audio Capture

The Linux service connects directly to PipeWire through the official Rust bindings and captures the default output sink monitor:

```text
PipeWire RT process callback
-> preallocated PCM frame pool
-> bounded SPSC capture queue
-> common packetizer
-> RTP/UDP sender
```

The background service exposes a stable `PcmSource` boundary:

- `--source auto`: prefer PipeWire on Linux and fall back to the test tone on failure.
- `--source pipewire`: require PipeWire and fail if it is unavailable.
- `--source tone`: use the test tone.
- `--pipewire-target <node>`: select a specific PipeWire node.

The process callback only copies PCM into reusable fixed-size frames and pushes them into an `rtrb` SPSC queue. A second SPSC queue returns consumed frames to the capture thread. Linux `eventfd` wakes the Tokio sender without polling or per-packet task allocation. If the bounded queue is full, the sender advances RTP sequence numbers and timestamps for the dropped capture interval instead of allowing latency to grow.

Linux builds require the PipeWire and libclang development packages (`libpipewire-0.3-dev` and `libclang-dev` on Debian and Ubuntu). Runtime capture no longer requires the `pw-record` command or any child process. Remaining Linux measurement work is real-time scheduling policy, negotiated quantum, and xrun instrumentation on representative systems.

### 5.3 Windows

Windows uses WASAPI loopback:

```text
IAudioClient loopback capture
-> PCM frames
-> common packetizer
```

### 5.4 macOS

macOS uses ScreenCaptureKit directly:

```text
ScreenCaptureKit system-audio capture
-> interleaved or planar Float32 to s16le conversion
-> PCM frames
-> common packetizer
```

The Rust service selects ScreenCaptureKit for `auto` on macOS. Capture callbacks reuse their conversion buffer and feed the same bounded SPSC packet queue used by the sender. Screen & System Audio Recording permission is mandatory; denial is returned through service diagnostics.

### 5.5 Desktop Shell

The current implementation uses `eframe/egui` with `tray-icon`, without a WebView.

Responsibilities:

- Start `lanpulse-service` automatically on launch.
- Default to English on first launch and persist the language selection. Linux bundles a subset Chinese font, while Windows and macOS use system Chinese fonts.
- Read `--json-events` startup events from the background service.
- Start and stop the service, poll `/api/status`, and disconnect devices on a dedicated worker thread. The UI thread performs no blocking I/O.
- Display the PIN, QR code, control address, control port, discovery port, audio format, target device, source, RTP packet count, and bytes sent.
- Display fallback states, background stderr, and local-time-zone logs, with actions to copy diagnostics and clear logs.
- Hide the window on close when the tray is available and the setting is enabled; exit normally if tray creation fails.
- Provide tray commands to show the window, change service state, and quit.
- Configure service startup, minimize-to-tray behavior, audio source, 5/10/20 ms packet duration, and control/discovery port ranges.

Desktop shell defaults:

```text
control_port_range: 4100..4199
discovery_port_range: 41000..41020
source: auto
packet_duration: 5 ms
```

## 6. Mobile

### 6.1 KMP Boundary

KMP is used for:

- Compose Multiplatform UI.
- Pairing flow.
- State machine.
- Settings contracts and localization.
- Shared protocol models.

KMP does not own the audio hot path:

- UDP reception can use platform implementations.
- LAN discovery and QR scanning use platform implementations.
- The jitter buffer can be common or native, but the audio callback must be native.
- Android and iOS audio output must use platform APIs.

### 6.2 Android

Android uses:

- Kotlin/Compose UI.
- A Chinese and English UI that defaults to English and persists the user's explicit selection.
- CameraX preview and pure-JVM ZXing decoding for offline on-device QR pairing without a Google Play Services runtime dependency.
- A Foreground Service with the `mediaPlayback` type.
- Low-latency `AudioTrack` with blocking writes on an I/O coroutine.
- A preallocated RTP packet pool, SPSC receive/recycle queues, and a fixed-slot reorder window with no per-packet payload allocation.
- A 15 ms initial adaptive jitter target that can shrink to 10 ms on stable networks and grow to 60 ms under jitter.
- Playback-head tracking and one-frame PCM insertion/removal for short-term clock-drift correction.
- A partial wake lock and high-performance Wi-Fi lock during playback.
- A low-importance persistent notification with a disconnect action.

Background playback must use a media-playback foreground service.

MediaSession, detailed jitter diagnostics, long-duration physical-device tuning, and a measured Oboe/AAudio comparison are still pending. `AudioTrack` remains the stable implementation because its low-latency performance mode reaches the platform fast path without adding a JNI boundary; Oboe should replace it only if device measurements show a material latency or underrun improvement.

Reference: [Android foreground service types](https://developer.android.com/develop/background-work/services/fgs/service-types)

### 6.3 Android Build Baseline

| Component | Version / target |
|-----------|------------------|
| Build JDK | Temurin Java 25.0.4 LTS |
| Android bytecode target | JVM 17 |
| Kotlin | 2.4.10 |
| Compose Multiplatform | 1.11.1 |
| Gradle | 9.5.0 |
| Android Gradle Plugin | 9.1.0 |
| CameraX | 1.6.1 |
| ZXing Core | 3.5.4, pure JVM |
| compileSdk / targetSdk | 36 |
| minSdk | 26 |

This is the newest mutually supported stable combination used by the repository. API 37 is installed for future testing but is not selected because AGP 9.1.0 does not yet declare compile SDK 37 support. Java 25 runs the build, while JVM 17 bytecode keeps Android toolchain and device compatibility stable.

### 6.4 iOS

iOS uses:

- The same Compose Multiplatform UI, pairing flow, validation, localization, and state controller as Android.
- A thin SwiftUI application shell that hosts the shared Compose view controller.
- Native Swift LAN discovery, HTTP control, VisionKit QR scanning, and RTP reception behind the KMP platform interface.
- AVAudioSession with the playback category.
- Background Audio capability.
- AVAudioEngine/AVAudioPlayerNode Float32 output with pooled buffers and vDSP conversion from RTP s16le.
- A 15 ms initial, 10-60 ms adaptive RTP reorder buffer, sequence-wrap handling, late/duplicate rejection, packet-loss silence, and three-second stream timeout recovery.
- AVAudioSession interruption, route-change, and media-services reset recovery plus `NWPathMonitor`-aware retry backoff.

Reference: [Configuring an app for media playback](https://developer.apple.com/documentation/avfoundation/configuring-your-app-for-media-playback)

## 7. Pairing and Security

### 7.1 Pairing Flow

```text
Desktop starts
  -> generate one-time PIN
  -> advertise service on LAN
  -> mobile enters PIN or scans QR code
  -> exchange device public keys
  -> derive device secret
  -> store paired device
```

### 7.2 Session Flow

```text
mobile connects to control channel
desktop verifies paired device
desktop sends media session configuration
both sides derive session key
desktop starts RTP/UDP media
```

### 7.3 Media Security

Minimum requirements:

- Every media packet carries a sequence number and authentication tag.
- The session key is derived from the pairing key.
- Recommended AEAD: ChaCha20-Poly1305 or AES-GCM.

The protocol can move to SRTP if standards compatibility becomes necessary. The first release can use a custom lightweight AEAD header.

## 8. LAN Discovery

The current implementation uses UDP probes and responses.

The mobile app or test tool sends the following message to a discovery port:

```text
LANPULSE_DISCOVER_V1
```

The desktop responds with:

```text
{
  "type": "lanpulse.desktop.v1",
  "name": "...",
  "control_url": "http://<lan-ip>:<port>",
  "control_port": <port>,
  "pin_required": true,
  "audio": {
    "sample_rate": 48000,
    "channels": 2,
    "sample_format": "s16le",
    "packet_ms": 5,
    "payload_type": 96,
    "ssrc": <u32>
  }
}
```

The default discovery-port range is `41000..41020`. The Android app scans this range so a single occupied discovery port cannot make the service unavailable.

## 9. Control Protocol

The current control channel uses HTTP JSON. The media channel uses UDP/RTP separately.

Implemented endpoints:

```text
GET  /
GET  /api/status
POST /api/connect
POST /api/heartbeat
POST /api/disconnect
```

Pairing request:

```json
{
  "pin": "123456",
  "udp_port": 5504,
  "client_id": "persistent-installation-id",
  "device_name": "phone"
}
```

The desktop uses the HTTP peer IP and `udp_port` as the RTP target. A successful response includes a per-connection `session_id`, which the mobile app includes in heartbeat and disconnect requests.

`/api/status` returns the active device name, RTP target, connection time, capture drops, media restarts, and the last media error. The current media model permits only one active receiver. A different `client_id` is rejected with HTTP 409 while a receiver's 15-second lease is active. Android and iOS refresh the lease every five seconds; an abandoned session expires automatically. The same installation may reconnect and replace its own UDP session, while a stale disconnect carrying the old `session_id` cannot terminate the newer session.

Desktop QR code content:

```text
lanpulse://pair?url=<percent-encoded-control-url>&pin=<pin>
```

The Android scanner validates the scheme, local IPv4 control address, and PIN format before applying scanned pairing data. The user explicitly starts the connection after reviewing the populated address and PIN.

## 10. Latency Budget

Target budget:

| Stage | Target |
|-------|--------|
| Desktop capture buffer | 5-10 ms |
| Packetization | 0-5 ms |
| Wi-Fi transport | 1-5 ms |
| Mobile jitter buffer | 10-30 ms |
| Mobile audio output buffer | 5-15 ms |
| Total | 20-60 ms |

Optimization order:

1. Reduce the desktop capture buffer.
2. Keep RTP datagrams below the path MTU by using 5 ms PCM packets by default.
3. Use low-latency audio output on the mobile platform.
4. Use an adaptive jitter buffer.
5. Allow Opus or a larger buffer when network quality is poor.

## 11. Optional Opus Mode

Opus is not the default low-latency mode, but remains an alternative.

Use cases:

- Poor Wi-Fi quality.
- Multiple devices.
- Lower power use.
- Lower bandwidth use.

Protocol:

- Opus over RTP, following RFC 7587.
- Configurable 5, 10, or 20 ms frames.

Reference: [RFC 7587](https://www.rfc-editor.org/rfc/rfc7587)

## 12. Why WebRTC Is Only an Alternative

WebRTC advantages:

- Standard encryption.
- Mature jitter buffer.
- NAT traversal.
- Mature browser and app SDKs.

Reasons it is not the primary design:

- LAN communication does not require NAT traversal.
- An audio-only use case does not require a complete calling stack.
- General-purpose WebRTC controls and buffering raise the minimum latency floor.
- A WebRTC sender is more complex to implement in Rust.

A future WebRTC compatibility mode can remain on the roadmap, but it does not block the primary implementation.

## 13. Validation Plan

### Current Desktop Validation

- `cargo check --workspace` passes.
- `cargo test --workspace` passes with 15 tests.
- `cargo build --release --workspace` passes.
- The startup event returns the actual control and discovery ports.
- When the control port is occupied, the service selects the next available port; verified with `4101`.
- The UDP discovery response returns the actual `control_url`.
- A valid PIN sent to `/api/connect` sets the RTP target.
- The Linux `auto` source changes to `pipewire` after a connection.
- The RTP packet and byte counters continuously increase.
- Native PipeWire capture runs in-process without a `pw-record` child process.
- The native desktop window is created successfully at `760x620` under X11.
- No `lanpulse-service` process remains after the desktop app exits.

### Current Android Build Validation

- `:androidApp:assembleDebug` passes and produces an installable APK.
- `:shared:allTests` passes, including protocol validation, RTP parsing and reordering, adaptive jitter control, playback-head wraparound, clock-drift direction, PCM frame adjustment, and SPSC reuse.
- `lintDebug` passes with no errors. The remaining old-target warning is intentional because API 36 is AGP 9.1.0's supported compile target while API 37 is installed ahead of toolchain support.
- Foreground/background and lock-screen behavior still require a connected physical Android device.

### Current iOS Build Validation

- Simulator and generic `iosArm64` device builds pass.
- Ten Swift XCTest tests pass, including adaptive jitter and real UDP-to-AVAudioEngine playback.
- Background, interruption, route-change, and long-duration behavior still require a physical iPhone.

### P1 Metrics

- Continuous Android foreground playback for 30 minutes.
- Continuous Android lock-screen playback for 30 minutes.
- End-to-end latency measured on the same Wi-Fi network.
- Target latency of 20-50 ms with 5 ms packets.
- Uninterrupted playback at 1% packet loss.
- Automatic recovery after a brief Wi-Fi switch.

### Latency Measurement

The first release uses an acoustic measurement:

1. Play a click sound on the computer.
2. Use the same phone or a second device to record both the computer and phone speakers.
3. Align the waveforms and calculate the time difference.

Protocol-level timestamp measurements can be added later.

## 14. Development Order

Completed:

1. Rust workspace.
2. RTP/UDP PCM packetizer and sender.
3. PIN-pairing control API.
4. UDP LAN discovery.
5. Native Linux system-output capture through PipeWire.
6. Native compact desktop window and system tray.
7. Release build.
8. Asynchronous desktop service management, settings, QR code, error states, diagnostics, and connected-device controls.
9. Default 5 ms RTP PCM packets that fit within a common MTU.
10. Android Compose discovery and pairing UI.
11. Android UDP/RTP parser, adaptive low-latency reorder buffer, clock-drift correction, and `AudioTrack` output.
12. Android media-playback foreground service, notification disconnect action, playback locks, and three-second reconnection.
13. Android CameraX/ZXing QR pairing scanner with validated local pairing data.
14. Chinese and English mobile UI with a persisted explicit language choice.
15. Preallocated SPSC queues and reusable packet/frame pools in the desktop capture and Android receive hot paths.
16. Native macOS ScreenCaptureKit audio capture and platform-aware desktop source settings.
17. iPhone app with the shared Compose UI plus native Swift discovery, QR scanning, pooled/adaptive RTP playback, audio-session recovery, network-aware reconnection, Background Audio, and simulator tests.
18. Renewable mobile-session leases and supervised desktop media-task recovery with operational counters.

Next steps:

1. Run foreground, background, lock-screen, Wi-Fi-switch, packet-loss, and acoustic latency tests on physical Android devices.
2. Add MediaSession, audio-focus handling, and persisted trusted devices.
3. Measure and tune the Android/iOS adaptive jitter and Android drift controllers, then compare `AudioTrack` with Oboe/AAudio only if the data warrants it.
4. Measure PipeWire quantum, xruns, and real-time scheduling behavior on representative Linux systems.
5. Add pairing keys plus media-packet authentication and encryption.
6. Add the optional Opus mode.
7. Add a Windows WASAPI loopback bridge.
8. Complete physical iPhone and permission-enabled macOS end-to-end latency, background, lock-screen, interruption, and long-duration testing.
