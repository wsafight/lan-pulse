# LanPulse for iPhone

The iPhone app hosts the same Compose Multiplatform UI used by Android. Swift remains responsible for local-network discovery, HTTP control, QR scanning, RTP, and audio playback.

Generate and open the Xcode project:

```bash
cd mobile/iosApp
xcodegen generate
open LanPulseIOS.xcodeproj
```

The Xcode build automatically creates and links the shared Kotlin framework. The app requires iOS 17 or newer. For device testing, select a Development Team in Xcode. Local network, camera, and Background Audio permissions are configured by the project specification.
