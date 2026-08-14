import Foundation

struct AudioConfig: Codable, Equatable, Sendable {
    let sampleRate: Int
    let channels: Int
    let sampleFormat: String
    let packetMs: Int
    let payloadType: UInt8
    let ssrc: UInt32

    enum CodingKeys: String, CodingKey {
        case sampleRate = "sample_rate"
        case channels
        case sampleFormat = "sample_format"
        case packetMs = "packet_ms"
        case payloadType = "payload_type"
        case ssrc
    }

    var framesPerPacket: Int { sampleRate * packetMs / 1_000 }
    var payloadBytes: Int { framesPerPacket * channels * MemoryLayout<Int16>.size }

    func validate() throws {
        guard sampleFormat == "s16le" else { throw LanPulseError.unsupportedAudio }
        guard (8_000...192_000).contains(sampleRate) else { throw LanPulseError.unsupportedAudio }
        guard (1...2).contains(channels) else { throw LanPulseError.unsupportedAudio }
        guard [5, 10, 20].contains(packetMs) else { throw LanPulseError.unsupportedAudio }
        guard (96...127).contains(payloadType) else { throw LanPulseError.unsupportedAudio }
        guard payloadBytes > 0 && payloadBytes <= 65_536 else {
            throw LanPulseError.unsupportedAudio
        }
    }
}

struct DiscoveryResponse: Codable, Sendable {
    let type: String
    let name: String
    let controlUrl: String
    let controlPort: Int
    let pinRequired: Bool
    let audio: AudioConfig

    enum CodingKeys: String, CodingKey {
        case type, name, audio
        case controlUrl = "control_url"
        case controlPort = "control_port"
        case pinRequired = "pin_required"
    }
}

struct DesktopEndpoint: Identifiable, Hashable, Sendable {
    let name: String
    let controlURL: URL
    let audio: AudioConfig?

    var id: String { controlURL.absoluteString.trimmingCharacters(in: CharacterSet(charactersIn: "/")) }

    func hash(into hasher: inout Hasher) {
        hasher.combine(id)
    }

    static func == (lhs: Self, rhs: Self) -> Bool {
        lhs.id == rhs.id
    }
}

struct ConnectRequest: Encodable, Sendable {
    let pin: String
    let udpPort: UInt16
    let clientId: String
    let deviceName: String

    enum CodingKeys: String, CodingKey {
        case pin
        case udpPort = "udp_port"
        case clientId = "client_id"
        case deviceName = "device_name"
    }
}

struct ConnectResponse: Decodable, Sendable {
    let ok: Bool
    let message: String
    let sessionId: String?
    let media: MediaConfig?

    enum CodingKeys: String, CodingKey {
        case ok, message, media
        case sessionId = "session_id"
    }
}

struct DisconnectRequest: Encodable, Sendable {
    let pin: String
    let sessionId: String?

    enum CodingKeys: String, CodingKey {
        case pin
        case sessionId = "session_id"
    }
}

struct HeartbeatRequest: Encodable, Sendable {
    let pin: String
    let sessionId: String

    enum CodingKeys: String, CodingKey {
        case pin
        case sessionId = "session_id"
    }
}

struct MediaConfig: Codable, Sendable {
    let targetIp: String
    let targetPort: UInt16
    let audio: AudioConfig

    enum CodingKeys: String, CodingKey {
        case targetIp = "target_ip"
        case targetPort = "target_port"
        case audio
    }
}

enum LanPulseError: LocalizedError, Sendable {
    case invalidAddress
    case invalidPin
    case desktopUnavailable
    case invalidResponse
    case unauthorized
    case deviceBusy
    case unsupportedAudio
    case audioStreamTimedOut
    case audioPlaybackFailed
    case cameraUnavailable
    case cameraPermissionRequired
    case server(String)

    var errorDescription: String? {
        switch self {
        case .invalidAddress: "Enter a valid local address and port."
        case .invalidPin: "Enter the six-digit PIN."
        case .desktopUnavailable: "The desktop is unavailable."
        case .invalidResponse: "The desktop returned an invalid response."
        case .unauthorized: "The PIN is incorrect."
        case .deviceBusy: "Another phone is already connected."
        case .unsupportedAudio: "The desktop provided an unsupported audio format."
        case .audioStreamTimedOut: "The audio stream timed out."
        case .audioPlaybackFailed: "The audio playback buffer could not be scheduled."
        case .cameraUnavailable: "The camera is unavailable."
        case .cameraPermissionRequired: "Camera permission is required to scan a pairing code."
        case .server(let message): message
        }
    }
}
