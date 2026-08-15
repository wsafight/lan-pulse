# LanPulse

English | [简体中文](./README_CN.md)

Low-latency LAN audio streaming from desktop to mobile. LanPulse captures system audio on the desktop and plays it through a native mobile app in the foreground, background, and on the lock screen.

## Documentation

| Document | Description |
|----------|-------------|
| [Product Requirements](./docs/product-requirements.md) | Product goals, use cases, release scope, and acceptance criteria |
| [Technical Design](./docs/technical-design.md) | Low-latency architecture, desktop and mobile clients, protocols, security, and roadmap |
| [Optimization Roadmap](./docs/optimization-roadmap.md) | Measured gaps, release blockers, and prioritized follow-up work |

## Architecture

```text
Desktop:
Rust native service
  - Linux PipeWire system-output capture
  - Windows WASAPI loopback
  - macOS ScreenCaptureKit/CoreAudio
  - LAN discovery / PIN pairing
  - UDP/RTP PCM audio sender
Rust native desktop shell
  - system tray
  - compact control window
  - service lifecycle management

Mobile:
  - shared KMP/Compose UI, pairing flow, validation, and state controller
  - Android AudioTrack + Foreground Service platform backend
  - iPhone Swift LAN/RTP backend + AVAudioEngine + Background Audio
  - shared JSON control and RTP/PCM wire protocol
```

## Why Not a Web App or WebRTC?

- Background and lock-screen playback is unreliable in mobile browsers because the OS may suspend JavaScript, WebSocket, and Web Audio.
- WebRTC is a strong fit for internet, NAT traversal, and calling, but it is not optimized for the lowest possible latency on a LAN.
- Raw PCM over UDP/RTP avoids audio encoding and offers a lower latency floor on a local network.
- Only a native mobile app can reliably use Android and iOS background media playback.

## Latency Targets

| Item | Target |
|------|--------|
| LAN audio latency | 20-50 ms |
| Reliable playback | Continuous playback in the foreground, background, and on the lock screen |
| Default audio format | PCM s16le, 48 kHz, stereo, 5 ms packets |
| Default media transport | UDP/RTP |
| Fallback mode | Opus/RTP for poor networks or lower bandwidth use |

## Security Principles

- Operate only on the local network by default.
- Generate a one-time PIN and QR code when the desktop service starts.
- Store a device key after pairing.
- Authenticate and encrypt media packets with a session key.
- Let the desktop user view and disconnect connected devices.

## References

- RTP: [RFC 3550](https://www.rfc-editor.org/rfc/rfc3550)
- Opus over RTP: [RFC 7587](https://www.rfc-editor.org/rfc/rfc7587)
- [Android foreground service media playback](https://developer.android.com/develop/background-work/services/fgs/service-types)
- [Apple media playback configuration](https://developer.apple.com/documentation/avfoundation/configuring-your-app-for-media-playback)

## Development Status

Implemented:

- Rust workspace.
- `desktop-service` background service.
- `desktop-app` native desktop shell with a compact window and system tray.
- `mobile/shared` KMP module with one Compose UI, pairing flow, validation, localization, and playback-state controller for Android and iPhone.
- Android platform client with LAN discovery, manual pairing, and native playback integration.
- Android foreground media-playback service with notification controls, a partial wake lock, and a high-performance Wi-Fi lock.
- Android UDP/RTP receiver and `AudioTrack` PCM playback with a dedicated audio-priority receive thread and two persisted playback modes: Immediate keeps only the current RTP packet, writes it non-blockingly into the platform's minimum output capacity, and does not wait for retransmission, correct clock drift, or rebuffer after an underrun; Adaptive starts at 120 ms, automatically operates from 40-450 ms, uses a 60 ms output target, grows quickly after network or scheduling disruption, and gradually returns toward the lowest stable delay. Both modes retain packet-loss handling, stream timeout detection, session resume after transient failures, network-aware reconnection, and segmented receive/dispatch/write diagnostics.
- Android QR pairing scanner built with CameraX and pure-JVM ZXing; scanned data is validated before it is applied.
- Simplified Chinese and English mobile UI with language and Android playback-mode choices in a dedicated settings page; English is the default on first launch, and selections are persisted.
- Simplified Chinese and English desktop UI; English is the default on first launch, and the selected language is persisted.
- Bundled subset Chinese font, independent of the operating system's font configuration.
- A single start/stop action in the main window and tray that follows service state.
- Asynchronous service management with error states, settings, a QR code, connected-device controls, and diagnostics.
- RTP packetizer.
- Native Linux PipeWire system-output capture with preallocated SPSC buffer queues; `auto` falls back to a test tone if capture is unavailable.
- Native macOS ScreenCaptureKit system-audio capture with reusable fixed-size Float32-to-s16le conversion buffers; `auto` selects ScreenCaptureKit on macOS.
- Preallocated Android RTP packet pools, SPSC handoff queues, and an incrementally pruned fixed-slot reorder window avoid per-packet `ByteArray` allocation, payload copies, and full-window scans.
- An iPhone app in `mobile/iosApp` that hosts the shared Compose UI and uses Swift for LAN discovery, HTTP control, QR scanning, pooled in-place RTP parsing, a 10-60 ms adaptive jitter buffer, bounded callback-driven AVAudioEngine playback, interruption/route recovery, network-aware reconnection, and Background Audio.
- 48 kHz stereo s16le PCM test source, with 5 ms RTP packets by default to avoid IP fragmentation on common MTUs.
- UDP/RTP media transmission.
- PIN-pairing control API.
- Single-receiver protection with a 15-second renewable lease: another phone is rejected while the active phone can safely reconnect, and abandoned sessions expire automatically.
- Supervised desktop media sending with bounded restart backoff, capture-drop/restart diagnostics, target caching, atomic batched packet counters, DSCP audio QoS, an optional 64-packet retransmit cache, and explicit capture-thread shutdown.
- Automatic control-port selection, defaulting to `4100..4199` in the desktop app.
- Automatic LAN discovery-port selection, defaulting to `41000..41020`.
- `lanpulse-discover` LAN discovery test tool.
- `lanpulse-recv` local RTP receiver test tool.

Verified:

- `cargo fmt --all`.
- `cargo check --workspace`.
- `cargo test --workspace`, with 18 tests passing on macOS.
- `cargo build --release --workspace`.
- Desktop window launches at `760x620` under X11.
- When the control port is occupied, the service selects another port; verified with port `4101`.
- LAN discovery responses contain the actual control port.
- RTP transmission starts after pairing through `/api/connect`.
- On Linux, the `auto` source switches to `pipewire` after pairing and the RTP packet counter increases.
- Native PipeWire capture runs in-process without a `pw-record` child process.
- `./gradlew :androidApp:assembleDebug :shared:allTests :androidApp:lintDebug` passes with Java 25.0.4 LTS, Kotlin 2.4.10, Gradle 9.5.0, AGP 9.1.0, Compose Multiplatform 1.11.1, and Android API 36.
- The minified, resource-shrunk unsigned Release APK builds successfully.
- The Android debug APK is generated at `mobile/androidApp/build/outputs/apk/debug/androidApp-debug.apk`.
- The iPhone app builds for the iOS Simulator with Xcode 26.4.
- Ten iOS Simulator tests pass, including adaptive-jitter coverage and a real UDP-to-AVAudioEngine playout test.
- The macOS ScreenCaptureKit probe reaches the system permission boundary and reports a clear TCC denial when capture access is not granted.

Current limitations:

- Android foreground/background playback is implemented but still needs physical-device latency, lock-screen, Wi-Fi-switch, and 30-minute stability validation.
- iPhone playback and QR scanning still need physical-device LAN, camera-permission, background/lock-screen, interruption/route-change, latency, and long-duration validation. Adaptive jitter thresholds still require device measurements.
- macOS system audio requires Screen & System Audio Recording permission. End-to-end capture remains unverified on this machine because TCC access is currently denied.
- Persisted trusted devices, pause/resume MediaSession controls, full diagnostic export, and long-duration physical-device tuning are not implemented yet.
- The media stream is not encrypted yet; only the PIN-based control layer is implemented.
- On Linux, the desktop tray depends on GTK/AppIndicator. Whether GNOME displays the tray icon depends on the installed desktop extensions.

## Development Commands

Linux builds require PipeWire development headers:

```bash
sudo apt install libpipewire-0.3-dev libclang-dev
```

Start the desktop window and system tray:

```bash
cargo build -p lanpulse-service --bin lanpulse-service
cargo run -p lanpulse-app
```

Start only the background service:

```bash
cargo run -p lanpulse-service --bin lanpulse-service
```

Test LAN discovery:

```bash
cargo run -p lanpulse-service --bin lanpulse-discover -- 127.0.0.1:41000
```

Test local RTP reception:

```bash
cargo run -p lanpulse-service --bin lanpulse-recv -- 127.0.0.1:5504
cargo run -p lanpulse-service --bin lanpulse-service -- --target 127.0.0.1:5504 --pin 123456 --source tone
```

Simulate mobile pairing:

```bash
curl -X POST http://127.0.0.1:<control-port>/api/connect \
  -H 'content-type: application/json' \
  -d '{"pin":"123456","udp_port":5504,"device_name":"local-test"}'
```

Create a release build:

```bash
cargo build --release --workspace
```

Build and verify the Android app:

```bash
cd mobile
./gradlew :androidApp:assembleDebug :shared:allTests :androidApp:lintDebug
adb install -r androidApp/build/outputs/apk/debug/androidApp-debug.apk
```

Generate, build, and test the iPhone app:

```bash
cd mobile/iosApp
xcodegen generate
xcodebuild -project LanPulseIOS.xcodeproj -scheme LanPulseIOS \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' test
```

Current Linux release binary sizes:

```text
target/release/lanpulse-app       15M
target/release/lanpulse-service  3.8M
target/release/lanpulse-discover 1.2M
target/release/lanpulse-recv     956K
```
