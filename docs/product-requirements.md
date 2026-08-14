# LanPulse Product Requirements

## 1. Product Positioning

LanPulse is an ultra-low-latency phone speaker for local networks.

The user starts the desktop service on a computer and pairs a phone on the same local network. The phone then acts as an external speaker and continuously plays the computer's system audio. The product prioritizes low latency and reliable background and lock-screen playback over the convenience of browser access.

## 2. Core Requirements

Must have:

- Continuous mobile playback, including in the background and on the lock screen.
- Low-latency playback on a local network.
- No dependency on a public internet service.
- No mobile web page as the final playback client.
- Direct development toward the final architecture instead of later migrating from a Web or WebRTC prototype.

Priority order:

1. Low latency.
2. Reliable background and lock-screen playback.
3. Simple LAN connectivity.
4. Secure pairing.
5. Cross-platform expansion.

## 3. Target Users

- People without computer speakers who want to use a phone as a temporary external speaker.
- People in dormitories, offices, or temporary workspaces who need low-latency computer audio playback.
- People dissatisfied with browser playback stopping on lock screen, unreliable Bluetooth pairing, or Bluetooth latency.

Not a priority:

- Listening to computer audio remotely over the internet.
- Multi-room music synchronization.
- Professional studio monitoring.
- Public video-conferencing relays.

## 4. Core Experience

### 4.1 First Use

1. The user starts LanPulse on the computer.
2. The desktop app displays a QR code and six-digit PIN.
3. The user opens the mobile app and scans the code or enters the PIN.
4. The mobile app pairs with the desktop app.
5. The user starts playback.
6. The phone begins playing the computer's system audio with low latency.

### 4.2 Daily Use

1. The desktop app automatically discovers previously paired phones after startup.
2. The mobile app reconnects with one action after it opens.
3. Playback continues when the phone is locked or the app enters the background.
4. The desktop app shows the connected device and latency status.

## 5. Product Form

### 5.1 Desktop App

The desktop app consists of a background service and a compact native window. The window displays status and controls pairing and service state. The system tray keeps the app resident and provides commands to show the window, change service state, and quit. Service start, stop, polling, and device-disconnection operations run on a worker thread so they do not block the UI.

Must have:

- Start and stop the audio service.
- Display the PIN.
- Display the current LAN address.
- Display the control port separately.
- Display the LAN discovery port.
- Display connected devices.
- Display the audio source.
- Display the RTP packet count and bytes sent.
- Display the QR code and PIN.
- Disconnect a device.
- Configure service startup, minimize-to-tray behavior, audio source, packet duration, and port ranges.
- Display service errors, copy diagnostics, and clear logs.
- Support Chinese and English, default to English on first launch, and remember the user's choice.
- Permit one active phone in the first release. Reject a different phone with a clear busy error, while allowing the same app installation to replace its own stale session during reconnection.

### 5.2 Mobile App

The mobile client is a native app, not a web page.

Must have:

- Pairing by QR code or PIN.
- Play and pause controls.
- Volume control.
- Latency mode selection.
- Connection status.
- Background and lock-screen playback.
- Automatic reconnection.
- An option to clear pairing data.
- Chinese and English UI, with English on first launch and the selected language persisted.

## 6. Low-Latency Strategy

The default transport is raw PCM over UDP/RTP on the local network:

```text
PCM s16le, 48 kHz, stereo
5 ms packets
adaptive jitter buffer: 10-20 ms
```

A 5 ms PCM payload is 960 bytes. With the 12-byte RTP header, the datagram is approximately 972 bytes, below the typical IPv4 MTU limit of about 1472 bytes for a UDP payload. Unlike a 10 ms, 1920-byte PCM payload, the default 5 ms packet avoids IP fragmentation and reduces packetization delay. Durations of 10 ms and 20 ms remain available for compatibility or power-saving modes.

Why Opus is not the default:

- Opus adds encoding and decoding latency.
- A local network has enough bandwidth for PCM.
- PCM has a shorter implementation path and a lower latency floor.

Opus remains an alternative for:

- Poor Wi-Fi conditions.
- Lower power or bandwidth use.
- Playback on multiple devices.

## 7. Security Strategy

Default security model:

- Listen only on the local network.
- Use a one-time PIN or QR code for initial pairing.
- Expire the PIN after a short period, such as 60 seconds.
- Generate a device key after successful pairing.
- Authenticate and encrypt the media stream with a session key.
- Let the desktop user remove paired devices.

Security goals:

- Prevent unknown devices on the same LAN from connecting without authorization.
- Prevent a basic packet capture from directly replaying the media stream.
- Avoid the complexity of a public-internet identity system.

## 8. First Release Scope

P1 includes:

- Linux desktop app.
- Android mobile app.
- LAN discovery.
- PIN pairing.
- UDP/RTP PCM transport.
- Android background and lock-screen playback.
- Latency, packet-loss, and buffer status.

P1 excludes:

- iOS.
- Windows.
- macOS.
- WebRTC.
- Public-internet access.
- Multi-phone synchronization.
- Audio recording.

## 9. Acceptance Criteria

Completed for the desktop app:

- The native compact window launches successfully.
- The system tray builds and provides show, service-state, and quit commands.
- Closing the window hides it when the tray is available and the setting is enabled. It exits normally if the tray is unavailable so the app cannot become inaccessible.
- The desktop app starts the background service automatically after launch.
- Service control and status polling do not block the UI thread.
- Log timestamps use the current system time zone.
- English is selected on first launch, and a manual language selection is persisted.
- The UI displays QR pairing data, the current device name and address, and a device-disconnect action.
- Settings cover the audio source, 5/10/20 ms packet durations, and control/discovery port ranges.
- The UI displays service errors and immediate operation feedback, captures background service stderr, copies diagnostics, and clears logs.
- The desktop app automatically selects a control port in `4100..4199` by default.
- The discovery service automatically selects a port in `41000..41020` by default.
- LAN discovery returns the actual control address and port.
- On Linux, PipeWire captures system output and sends RTP after pairing.
- Default 5 ms RTP PCM packets fit a common MTU and avoid IP fragmentation.

P1 must pass:

- A phone and computer on the same Wi-Fi network can pair.
- The phone plays the computer's system audio in the foreground.
- Playback continues after the mobile app enters the background.
- Playback continues after the phone is locked.
- End-to-end latency is 20-50 ms on a stable LAN.
- Reconnection begins within three seconds after a disconnection.
- Unpaired devices cannot play the stream.
- The mobile app clearly shows a disconnected state after the desktop app stops.

## 10. Roadmap

### P1: Linux + Android

- Rust desktop background service.
- Native PipeWire capture.
- KMP Android app.
- Native Android audio output.
- UDP/RTP PCM.
- PIN pairing.

### P2: Experience Improvements

- Latency charts.
- Device history and trusted-device management.
- Login startup installation and integration.
- Opus bandwidth-saving mode.

### P3: iOS

- Native iOS audio output.
- AVAudioSession.
- Background Audio.
- Shared KMP pairing and playback UI.

### P4: Windows and macOS

- Windows WASAPI loopback.
- macOS ScreenCaptureKit/CoreAudio.
- Platform installers.

### P5: Advanced Transport

- Adaptive-buffer presets and advanced diagnostics.
- Forward error correction.
- Multiple devices.
- WebRTC compatibility mode.
