# LanPulse

[English](./README.md) | 简体中文

从桌面端到移动端的低延迟局域网音频传输。LanPulse 在电脑上采集系统声音，并通过手机原生 App 在前台、后台和锁屏状态下持续播放。

## 文档

| 文档 | 说明 |
|------|------|
| [产品文档](./docs/product-requirements.md) | 产品目标、用户场景、版本范围、验收标准 |
| [技术方案](./docs/technical-design.md) | 低延迟架构、当前选型、桌面端、手机端、协议、安全和路线 |
| [优化路线](./docs/optimization-roadmap.md) | 实测缺口、发布阻断项和后续优化优先级 |

## 最终路线

```text
桌面端：
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

手机端：
  - Android 与 iPhone 共用 KMP/Compose UI、配对流程、校验和状态控制器
  - Android AudioTrack + Foreground Service 平台后端
  - iPhone Swift 局域网/RTP 后端 + AVAudioEngine + Background Audio
  - 共用 JSON 控制协议和 RTP/PCM 线协议
```

## 为什么不用网页或 WebRTC 做主路线

- 移动网页后台和锁屏播放不可控，系统可能挂起 JS、WebSocket 和 Web Audio。
- WebRTC 很适合公网/NAT/通话场景，但局域网内最低延迟不是它的强项。
- 局域网原始 PCM over UDP/RTP 不需要音频编码，延迟下限更低。
- 手机端原生 App 才能稳定使用 Android/iOS 的后台媒体播放能力。

## 低延迟目标

| 项目 | 目标 |
|------|------|
| 局域网音频延迟 | 20-50ms |
| 稳定播放目标 | 前台、后台、锁屏持续播放 |
| 默认音频格式 | PCM s16le, 48kHz, stereo, 5ms packet |
| 默认媒体传输 | UDP/RTP |
| 备选模式 | Opus/RTP，用于弱网或省带宽 |

## 安全原则

- 默认只在局域网工作。
- 桌面端启动后生成一次性 PIN/二维码。
- 配对后保存设备密钥。
- 媒体包使用会话密钥认证和加密。
- 桌面端可查看和踢掉已连接设备。

## 参考

- RTP: [RFC 3550](https://www.rfc-editor.org/rfc/rfc3550)
- Opus over RTP: [RFC 7587](https://www.rfc-editor.org/rfc/rfc7587)
- [Android 前台服务媒体播放](https://developer.android.com/develop/background-work/services/fgs/service-types)
- [Apple 媒体播放配置](https://developer.apple.com/documentation/avfoundation/configuring-your-app-for-media-playback)

## 当前开发状态

已落地：

- Rust workspace。
- `desktop-service` 桌面后台服务。
- `desktop-app` 原生桌面壳，小窗口 + 系统托盘。
- `mobile/shared` KMP 模块，为 Android 和 iPhone 提供同一份 Compose UI、配对流程、校验、多语言和播放状态控制器。
- Android 平台客户端，包含局域网发现、手动配对和原生播放集成。
- Android `mediaPlayback` 前台服务、通知栏断开操作、局部唤醒锁和 Wi-Fi 高性能锁。
- Android UDP/RTP 接收和 `AudioTrack` PCM 播放，使用独立音频优先级收包线程，并提供两种可持久化的播放方式：即时播放只保留当前有效 RTP 载荷，向系统允许的最小输出容量执行非阻塞写入，不重排或分析丢包、不等待快速重传、不修正时钟漂移，欠载后也不重新缓冲；自适应播放从 120 ms 开始，在 40-450 ms 内自动调整，输出目标为 60 ms，网络或调度异常后快速增大，稳定后逐步回落到可稳定播放的最低延迟。自适应播放保留丢包处理，两种方式均保留流超时检测、临时故障后的 session resume、网络感知重连，以及收包/写入诊断。
- Android 使用 CameraX 和纯 JVM ZXing 离线扫描配对二维码，并在应用扫描结果前校验数据。
- 移动端使用独立设置页管理简体中文、英文和 Android 播放方式；首次默认英文，并记住用户选择。
- 桌面壳支持简体中文和英文，首次默认英文并记住用户选择。
- 桌面壳内置精简中文字体，不依赖操作系统字体配置。
- 主窗口和托盘使用随服务状态变化的单一启停操作。
- 桌面壳异步管理服务，支持异常状态、设置、二维码、当前设备断开和诊断信息。
- RTP packetizer。
- Linux 原生 PipeWire 系统输出采集和预分配 SPSC 缓冲队列，`auto` 模式不可用时回退测试音。
- macOS 原生 ScreenCaptureKit 系统音频采集，复用固定大小的 Float32 到 s16le 转换缓冲；macOS 的 `auto` 模式自动选择 ScreenCaptureKit。
- Android 使用预分配 RTP 包池、SPSC 交接队列和增量清理的固定槽乱序窗口，避免逐包分配 `ByteArray`、复制负载和全窗口扫描。
- `mobile/iosApp` iPhone App 承载共享 Compose UI；Swift 负责局域网发现、HTTP 控制、二维码扫描、池化原位 RTP 解析、10-60 ms 自适应抖动缓冲、有界回调驱动的 AVAudioEngine 播放、音频中断/路由恢复、网络感知重连和 Background Audio。
- 48kHz stereo s16le PCM 测试音源，默认 5ms RTP 包以避免常见 MTU 下的 IP 分片。
- UDP/RTP 媒体发送。
- PIN 配对控制接口。
- 带 15 秒可续租租约的单接收端占用保护：另一台手机会被拒绝，当前手机可安全重连，异常退出遗留的会话会自动过期。
- 桌面媒体发送任务带有界退避自愈、捕获丢帧/重启诊断、目标地址缓存、原子发送计数批量提交、DSCP 音频 QoS、可选 64 包快速重传缓存和显式捕获线程回收。
- 控制端口自动选择：桌面端默认 `4100..4199`。
- 局域网发现端口自动选择：默认 `41000..41020`。
- `lanpulse-discover` 局域网发现测试工具。
- `lanpulse-recv` 本地 RTP 接收测试工具。

已验证：

- `cargo fmt --all`。
- `cargo check --workspace`。
- `cargo test --workspace`，macOS 上 18 个测试通过。
- `cargo build --release --workspace`。
- 桌面窗口可启动，X11 下窗口尺寸 `760x620`。
- 控制端口被占用时自动换端口，实测启动到 `4101`。
- 局域网发现响应返回实际控制端口。
- 通过 `/api/connect` 配对后开始发送 RTP。
- Linux `auto` 音源配对后进入 `pipewire`，RTP 包计数增长。
- 原生 PipeWire 采集在服务进程内运行，不再创建 `pw-record` 子进程。
- Java 25.0.4 LTS、Kotlin 2.4.10、Gradle 9.5.0、AGP 9.1.0、Compose Multiplatform 1.11.1、Android API 36 组合下，Android APK、共享测试和 lint 构建通过。
- 已通过 R8 和资源压缩的未签名 Release APK 构建。
- Android Debug APK 输出到 `mobile/androidApp/build/outputs/apk/debug/androidApp-debug.apk`。
- iPhone App 已使用 Xcode 26.4 通过 iOS Simulator 构建。
- iOS Simulator 10 个测试通过，包含自适应抖动测试和真实 UDP 到 AVAudioEngine 播放测试。
- macOS ScreenCaptureKit 探针能够到达系统权限边界；未授权时会明确报告 TCC 拒绝。

当前限制：

- Android 前后台播放已实现，但仍需真机完成延迟、锁屏、Wi-Fi 切换和 30 分钟稳定性验证。
- iPhone 播放与扫码仍需真机完成局域网、相机权限、后台/锁屏、中断/路由变化、延迟和长时间稳定性验证；自适应抖动阈值仍需真机测量校准。
- macOS 系统音频需要“屏幕与系统音频录制”权限；本机当前被 TCC 拒绝，因此尚未完成真实音频端到端验证。
- 可信设备持久化、MediaSession 暂停/继续控制、完整诊断导出和长时间真机调优尚未实现。
- 媒体流还未加密，当前只实现 PIN 控制层。
- Linux 桌面壳托盘依赖 GTK/AppIndicator；GNOME 是否显示托盘图标取决于桌面环境扩展。

## 开发命令

Linux 构建需要 PipeWire 开发头文件：

```bash
sudo apt install libpipewire-0.3-dev libclang-dev
```

启动桌面端小窗口 + 系统托盘：

```bash
cargo build -p lanpulse-service --bin lanpulse-service
cargo run -p lanpulse-app
```

单独启动后台服务：

```bash
cargo run -p lanpulse-service --bin lanpulse-service
```

局域网发现测试：

```bash
cargo run -p lanpulse-service --bin lanpulse-discover -- 127.0.0.1:41000
```

本地 RTP 接收测试：

```bash
cargo run -p lanpulse-service --bin lanpulse-recv -- 127.0.0.1:5504
cargo run -p lanpulse-service --bin lanpulse-service -- --target 127.0.0.1:5504 --pin 123456 --source tone
```

模拟手机配对：

```bash
curl -X POST http://127.0.0.1:<control-port>/api/connect \
  -H 'content-type: application/json' \
  -d '{"pin":"123456","udp_port":5504,"device_name":"local-test"}'
```

release 构建：

```bash
cargo build --release --workspace
```

为当前 Linux 用户构建并安装优化后的桌面应用：

```bash
./scripts/install-linux.sh
```

该脚本会把应用、服务、桌面启动项和图标安装到 `~/.local`。仅供开发使用的
`lanpulse-discover` 和 `lanpulse-recv` 工具不会被安装。

构建并检查 Android App：

```bash
cd mobile
./gradlew :androidApp:assembleDebug :shared:allTests :androidApp:lintDebug
adb install -r androidApp/build/outputs/apk/debug/androidApp-debug.apk
```

生成、构建并测试 iPhone App：

```bash
cd mobile/iosApp
xcodegen generate
xcodebuild -project LanPulseIOS.xcodeproj -scheme LanPulseIOS \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' test
```

当前发布产物大小：

```text
target/release/lanpulse-app                                                   9.2M
target/release/lanpulse-service                                              2.3M
mobile/androidApp/build/outputs/apk/release/androidApp-release-unsigned.apk  1.9M
```

两个 Linux 二进制压缩后合计约 3.9 MB。Android Debug APK 约为 17 MB，原因是
包含 Compose 开发工具；Release 构建已经启用代码和资源裁剪。
