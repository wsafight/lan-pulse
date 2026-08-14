# LanPulse Release Checklist

This checklist is intentionally explicit about artifacts and checksums. Do not publish a build that skips the license, signing, or security notes.

## Required Decisions

- Choose and add the repository `LICENSE`.
- Decide whether the first public release ships unsigned side-load APKs, signed APKs, AABs, or a combination.
- Decide where signing keys live and who can access them.
- Decide whether GitHub Releases is the only distribution channel for the first public build.

## Automated Checks

- `cargo fmt --all -- --check`
- `cargo test --workspace`
- `cargo llvm-cov --workspace --summary-only`
- `cd mobile && ./gradlew :shared:testAndroidHostTest :shared:testAndroid`
- `bash -n scripts/linux-netem.sh`
- `bash scripts/linux-netem-test.sh`

## Manual Validation

- Record a Linux-to-Android test using `docs/test-record-template.md`.
- Validate foreground, background, lock-screen, disconnect, reconnect, and netem loss/pause scenarios.
- Confirm the Android notification, lock-screen controls, and media button behavior match the same playback state.
- Confirm protocol incompatibility, expired PIN, failed PIN limit, and busy-device errors are visible to the user.
- Confirm the app does not auto-play after explicit disconnect.

## Desktop Artifacts

- Build release binaries with `cargo build --release --workspace`.
- Capture artifact names, OS, architecture, and commit SHA.
- Generate SHA-256 checksums for each binary or package.
- Smoke-test service startup, discovery, QR pairing payload, and diagnostics copy.
- For Linux, test package install and uninstall on a clean VM.
- For macOS, verify Screen & System Audio Recording permission flow before calling it production-ready.
- For Windows, do not publish a desktop receiver until WASAPI loopback capture is implemented and tested.

## Android Artifacts

- Build debug APK only for internal testing.
- Build release APK/AAB only after signing configuration is finalized.
- Generate SHA-256 checksums for APK/AAB artifacts.
- Install on a clean Android device with `adb install -r`.
- Verify first-run language, notification permission, camera permission, discovery, QR scan, manual URL, connect, playback, and disconnect.
- Record package size and fail the release if it unexpectedly grows without a known dependency reason.

## Release Notes

- Include commit SHA and build date.
- List supported platforms honestly.
- State that media transport is not encrypted until media AEAD is implemented.
- State measured latency only when a matching test record exists.
- List known gaps: trusted devices, media encryption, Windows capture, and remaining long-duration device validation.

## Publication

- Upload artifacts and `.sha256` files together.
- Attach the release checklist result and sanitized test records.
- Verify every download link and checksum from a clean browser session.
- Tag only after artifacts and notes are final.
