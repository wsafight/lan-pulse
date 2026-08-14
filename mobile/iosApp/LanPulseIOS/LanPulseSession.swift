import Combine
import Foundation
import Network
import Shared
import UIKit

@MainActor
final class LanPulseSession: NSObject, ObservableObject, @MainActor IosLanPulseBackend {
    private weak var observer: IosPlaybackObserver?
    private var receiver: RTPAudioReceiver?
    private var connectionTask: Task<Void, Never>?
    private var heartbeatTask: Task<Void, Never>?
    private var connectionToken = UUID()
    private var activeEndpoint: DesktopEndpoint?
    private var activePIN: String?
    private var activeSessionID: String?
    private var pairingScanner: PairingScanner?
    private let defaults: UserDefaults
    private let clientID: String
    private var languageCode: String
    private let pathMonitor = NWPathMonitor()
    private let pathQueue = DispatchQueue(label: "com.lanpulse.ios.path")
    private var networkAvailable = true
    private var networkGeneration: UInt64 = 0

    var initialLanguageCode: String { languageCode }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        let clientKey = "lanpulse.client-id"
        if let existing = defaults.string(forKey: clientKey) {
            clientID = existing
        } else {
            let created = UUID().uuidString.lowercased()
            defaults.set(created, forKey: clientKey)
            clientID = created
        }

        let languageKey = "lanpulse.language"
        languageCode = defaults.string(forKey: languageKey)
            ?? (Locale.preferredLanguages.first?.hasPrefix("zh") == true ? "zh-CN" : "en")
        super.init()
        pathMonitor.pathUpdateHandler = { [weak self] path in
            Task { @MainActor [weak self] in
                guard let self else { return }
                let available = path.status == .satisfied && (
                    path.usesInterfaceType(.wifi) || path.usesInterfaceType(.wiredEthernet)
                )
                if available && !networkAvailable {
                    networkGeneration &+= 1
                }
                networkAvailable = available
            }
        }
        pathMonitor.start(queue: pathQueue)
    }

    deinit {
        pathMonitor.cancel()
    }

    func setObserver(observer: IosPlaybackObserver) {
        self.observer = observer
    }

    func discover(completion: @escaping ([IosDesktopEndpoint]?, String?) -> Void) {
        Task {
            do {
                let endpoints = try await LanDiscovery.discover().map(\.sharedEndpoint)
                completion(endpoints, nil)
            } catch {
                completion(nil, message(for: error))
            }
        }
    }

    func connect(endpoint: IosDesktopEndpoint, pin: String, languageCode: String) {
        self.languageCode = languageCode
        let nativeEndpoint: DesktopEndpoint
        do {
            nativeEndpoint = try endpoint.nativeEndpoint()
        } catch {
            observer?.onFailed(message: message(for: error))
            return
        }

        disconnectLocally()
        let token = UUID()
        connectionToken = token
        activeEndpoint = nativeEndpoint
        activePIN = pin
        observer?.onConnecting(desktopName: nativeEndpoint.name)
        connectionTask = Task {
            do {
                try await establish(endpoint: nativeEndpoint, pin: pin, token: token)
            } catch is CancellationError {
                return
            } catch {
                guard token == connectionToken else { return }
                observer?.onFailed(message: message(for: error))
            }
        }
    }

    func disconnect() {
        let endpoint = activeEndpoint
        let disconnectPIN = activePIN
        let sessionID = activeSessionID
        connectionToken = UUID()
        disconnectLocally()
        observer?.onIdle()
        guard let endpoint, let disconnectPIN else { return }
        Task {
            await ControlAPI.disconnect(
                controlURL: endpoint.controlURL,
                pin: disconnectPIN,
                sessionId: sessionID
            )
        }
    }

    func scanPairingCode(languageCode: String) {
        self.languageCode = languageCode
        let scanner = PairingScanner(
            onCode: { [weak self] value in
                self?.observer?.onPairingCode(value: value)
                self?.pairingScanner = nil
            },
            onFailure: { [weak self] error in
                guard let self else { return }
                self.observer?.onPairingScanFailed(message: self.message(for: error))
                self.pairingScanner = nil
            },
            onCancel: { [weak self] in
                self?.pairingScanner = nil
            }
        )
        pairingScanner = scanner
        scanner.start()
    }

    func saveLanguage(languageCode: String) {
        self.languageCode = languageCode
        defaults.set(languageCode, forKey: "lanpulse.language")
    }

    private func establish(endpoint: DesktopEndpoint, pin: String, token: UUID) async throws {
        let nextReceiver = try RTPAudioReceiver()
        do {
            guard token == connectionToken else {
                nextReceiver.stop()
                throw CancellationError()
            }
            receiver = nextReceiver
            let response = try await ControlAPI.connect(
                endpoint: endpoint,
                pin: pin,
                udpPort: nextReceiver.localPort,
                clientId: clientID,
                deviceName: UIDevice.current.name
            )
            guard token == connectionToken else {
                nextReceiver.stop()
                throw CancellationError()
            }
            guard response.ok, let media = response.media, let sessionID = response.sessionId else {
                throw LanPulseError.server(response.message)
            }
            try nextReceiver.start(
                audio: media.audio,
                onStats: { [weak self] stats in
                    Task { @MainActor in self?.apply(stats: stats, token: token) }
                },
                onFailure: { [weak self] error in
                    Task { @MainActor in self?.handleReceiverFailure(error, token: token) }
                }
            )
            activeSessionID = sessionID
            startHeartbeat(endpoint: endpoint, pin: pin, sessionID: sessionID, token: token)
            let initialBufferPackets = max(1, (15 + media.audio.packetMs - 1) / media.audio.packetMs)
            observer?.onPlaying(
                desktopName: endpoint.name,
                packetsReceived: 0,
                packetsLost: 0,
                bufferMs: Int32(media.audio.packetMs * initialBufferPackets)
            )
        } catch {
            nextReceiver.stop()
            if receiver === nextReceiver {
                receiver = nil
            }
            throw error
        }
    }

    private func apply(stats: ReceiverStats, token: UUID) {
        guard token == connectionToken, let endpoint = activeEndpoint else { return }
        observer?.onPlaying(
            desktopName: endpoint.name,
            packetsReceived: Int64(clamping: stats.packetsReceived),
            packetsLost: Int64(clamping: stats.packetsLost),
            bufferMs: Int32(clamping: stats.bufferMs)
        )
    }

    private func handleReceiverFailure(_ error: Error, token: UUID) {
        guard token == connectionToken,
              let endpoint = activeEndpoint,
              let pin = activePIN
        else { return }
        receiver?.stop()
        receiver = nil
        heartbeatTask?.cancel()
        heartbeatTask = nil
        beginReconnect(endpoint: endpoint, pin: pin, token: token, reason: error)
    }

    private func disconnectLocally() {
        connectionTask?.cancel()
        connectionTask = nil
        heartbeatTask?.cancel()
        heartbeatTask = nil
        receiver?.stop()
        receiver = nil
        activeEndpoint = nil
        activePIN = nil
        activeSessionID = nil
    }

    private func startHeartbeat(endpoint: DesktopEndpoint, pin: String, sessionID: String, token: UUID) {
        heartbeatTask?.cancel()
        heartbeatTask = Task {
            while !Task.isCancelled, token == connectionToken {
                do {
                    try await Task.sleep(for: .seconds(5))
                    guard !Task.isCancelled, token == connectionToken else { return }
                    try await ControlAPI.heartbeat(
                        controlURL: endpoint.controlURL,
                        pin: pin,
                        sessionId: sessionID
                    )
                } catch is CancellationError {
                    return
                } catch {
                    // RTP timeout drives reconnection; a transient control failure should not stop audio.
                }
            }
        }
    }

    private func beginReconnect(
        endpoint: DesktopEndpoint,
        pin: String,
        token: UUID,
        reason: Error
    ) {
        observer?.onReconnecting(desktopName: endpoint.name, reason: message(for: reason))
        connectionTask?.cancel()
        connectionTask = Task {
            var attempt = 0
            while !Task.isCancelled, token == connectionToken {
                do {
                    try await waitForUsableNetwork(token: token)
                    if attempt > 0 {
                        try await waitForRetryDelay(attempt: attempt, token: token)
                    }
                    guard !Task.isCancelled, token == connectionToken else { return }
                    try await establish(endpoint: endpoint, pin: pin, token: token)
                    return
                } catch is CancellationError {
                    return
                } catch {
                    guard token == connectionToken else { return }
                    if !isRetryable(error) {
                        observer?.onFailed(message: message(for: error))
                        return
                    }
                    attempt += 1
                    observer?.onReconnecting(
                        desktopName: endpoint.name,
                        reason: message(for: error)
                    )
                }
            }
        }
    }

    private func waitForUsableNetwork(token: UUID) async throws {
        while !networkAvailable {
            guard !Task.isCancelled, token == connectionToken else { throw CancellationError() }
            try await Task.sleep(for: .milliseconds(200))
        }
    }

    private func waitForRetryDelay(attempt: Int, token: UUID) async throws {
        let exponent = min(attempt - 1, 4)
        var remainingMs = min(500 * (1 << exponent), 5_000)
        let observedGeneration = networkGeneration
        while remainingMs > 0 {
            guard !Task.isCancelled, token == connectionToken else { throw CancellationError() }
            if !networkAvailable {
                try await waitForUsableNetwork(token: token)
                return
            }
            if networkGeneration != observedGeneration {
                return
            }
            let interval = min(remainingMs, 200)
            try await Task.sleep(for: .milliseconds(interval))
            remainingMs -= interval
        }
    }

    private func isRetryable(_ error: Error) -> Bool {
        guard let error = error as? LanPulseError else { return true }
        switch error {
        case .invalidAddress, .invalidPin, .unauthorized, .deviceBusy, .unsupportedAudio:
            return false
        case .desktopUnavailable, .invalidResponse, .audioStreamTimedOut,
             .audioPlaybackFailed, .server, .cameraUnavailable, .cameraPermissionRequired:
            return true
        }
    }

    private func message(for error: Error) -> String {
        guard languageCode == "zh-CN", let error = error as? LanPulseError else {
            return error.localizedDescription
        }
        switch error {
        case .invalidAddress: return "请输入有效的局域网地址和端口。"
        case .invalidPin: return "请输入 6 位 PIN。"
        case .desktopUnavailable: return "无法访问桌面端。"
        case .invalidResponse: return "桌面端返回了无效响应。"
        case .unauthorized: return "PIN 不正确。"
        case .deviceBusy: return "已有其他手机连接。"
        case .unsupportedAudio: return "桌面端提供了不支持的音频格式。"
        case .audioStreamTimedOut: return "音频流已超时。"
        case .audioPlaybackFailed: return "音频播放缓冲失败。"
        case .cameraUnavailable: return "相机不可用。"
        case .cameraPermissionRequired: return "需要相机权限才能扫描配对二维码。"
        case .server(let message): return message
        }
    }
}

private extension DesktopEndpoint {
    var sharedEndpoint: IosDesktopEndpoint {
        IosDesktopEndpoint(
            name: name,
            controlUrl: controlURL.absoluteString,
            sampleRate: Int32(audio?.sampleRate ?? 0),
            channels: Int32(audio?.channels ?? 0),
            sampleFormat: audio?.sampleFormat ?? "",
            packetMs: Int32(audio?.packetMs ?? 0),
            payloadType: Int32(audio?.payloadType ?? 0),
            ssrc: Int64(audio?.ssrc ?? 0),
            hasAudio: audio != nil
        )
    }
}

private extension IosDesktopEndpoint {
    func nativeEndpoint() throws -> DesktopEndpoint {
        guard let controlURL = URL(string: controlUrl) else { throw LanPulseError.invalidAddress }
        let config: AudioConfig?
        if hasAudio {
            guard let payloadType = UInt8(exactly: payloadType),
                  let ssrc = UInt32(exactly: ssrc)
            else { throw LanPulseError.unsupportedAudio }
            config = AudioConfig(
                sampleRate: Int(sampleRate),
                channels: Int(channels),
                sampleFormat: sampleFormat,
                packetMs: Int(packetMs),
                payloadType: payloadType,
                ssrc: ssrc
            )
        } else {
            config = nil
        }
        return DesktopEndpoint(name: name, controlURL: controlURL, audio: config)
    }
}
