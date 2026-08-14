import Darwin
import Foundation

enum LanDiscovery {
    private static let probe = Data("LANPULSE_DISCOVER_V1".utf8)

    static func discover() async throws -> [DesktopEndpoint] {
        try await Task.detached(priority: .userInitiated) {
            try discoverBlocking()
        }.value
    }

    private static func discoverBlocking() throws -> [DesktopEndpoint] {
        let descriptor = Darwin.socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP)
        guard descriptor >= 0 else { throw LanPulseError.desktopUnavailable }
        defer { Darwin.close(descriptor) }

        var enabled: Int32 = 1
        setsockopt(descriptor, SOL_SOCKET, SO_BROADCAST, &enabled, socklen_t(MemoryLayout.size(ofValue: enabled)))
        setsockopt(descriptor, SOL_SOCKET, SO_REUSEADDR, &enabled, socklen_t(MemoryLayout.size(ofValue: enabled)))
        var timeout = timeval(tv_sec: 0, tv_usec: 120_000)
        setsockopt(descriptor, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout.size(ofValue: timeout)))

        for port in 41_000...41_020 {
            var address = sockaddr_in()
            address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
            address.sin_family = sa_family_t(AF_INET)
            address.sin_port = in_port_t(port).bigEndian
            address.sin_addr = in_addr(s_addr: inet_addr("255.255.255.255"))
            probe.withUnsafeBytes { bytes in
                withUnsafePointer(to: &address) { pointer in
                    pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { socketAddress in
                        _ = Darwin.sendto(
                            descriptor,
                            bytes.baseAddress,
                            bytes.count,
                            0,
                            socketAddress,
                            socklen_t(MemoryLayout<sockaddr_in>.size)
                        )
                    }
                }
            }
        }

        let deadline = DispatchTime.now().uptimeNanoseconds + 1_200_000_000
        let decoder = JSONDecoder()
        var results: [String: DesktopEndpoint] = [:]
        var buffer = [UInt8](repeating: 0, count: 4_096)
        while DispatchTime.now().uptimeNanoseconds < deadline {
            let count = Darwin.recv(descriptor, &buffer, buffer.count, 0)
            guard count > 0 else { continue }
            guard let response = try? decoder.decode(
                DiscoveryResponse.self,
                from: Data(buffer.prefix(count))
            ), response.type == "lanpulse.desktop.v1",
                let url = URL(string: response.controlUrl)
            else { continue }
            let endpoint = DesktopEndpoint(name: response.name, controlURL: url, audio: response.audio)
            results[endpoint.id] = endpoint
        }
        return results.values.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    }
}

enum ControlAPI {
    static func connect(
        endpoint: DesktopEndpoint,
        pin: String,
        udpPort: UInt16,
        clientId: String,
        deviceName: String
    ) async throws -> ConnectResponse {
        let payload = try JSONEncoder().encode(
            ConnectRequest(pin: pin, udpPort: udpPort, clientId: clientId, deviceName: deviceName)
        )
        let data = try await post(endpoint.controlURL, path: "/api/connect", payload: payload)
        guard let response = try? JSONDecoder().decode(ConnectResponse.self, from: data) else {
            throw LanPulseError.invalidResponse
        }
        return response
    }

    static func disconnect(controlURL: URL, pin: String, sessionId: String?) async {
        guard let payload = try? JSONEncoder().encode(
            DisconnectRequest(pin: pin, sessionId: sessionId)
        ) else {
            return
        }
        _ = try? await post(controlURL, path: "/api/disconnect", payload: payload)
    }

    static func heartbeat(controlURL: URL, pin: String, sessionId: String) async throws {
        let payload = try JSONEncoder().encode(HeartbeatRequest(pin: pin, sessionId: sessionId))
        _ = try await post(controlURL, path: "/api/heartbeat", payload: payload)
    }

    private static func post(_ controlURL: URL, path: String, payload: Data) async throws -> Data {
        guard var components = URLComponents(url: controlURL, resolvingAgainstBaseURL: false) else {
            throw LanPulseError.invalidAddress
        }
        components.path = controlURL.path.trimmingCharacters(in: CharacterSet(charactersIn: "/")) + path
        guard let url = components.url else { throw LanPulseError.invalidAddress }

        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.httpBody = payload
        request.timeoutInterval = 3
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")

        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await URLSession.shared.data(for: request)
        } catch {
            throw LanPulseError.desktopUnavailable
        }
        guard let http = response as? HTTPURLResponse else { throw LanPulseError.invalidResponse }
        switch http.statusCode {
        case 200...299: return data
        case 401: throw LanPulseError.unauthorized
        case 409: throw LanPulseError.deviceBusy
        default:
            let message = String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines)
            throw LanPulseError.server(message?.isEmpty == false ? message! : "Desktop returned HTTP \(http.statusCode).")
        }
    }
}

func manualEndpoint(from input: String) throws -> DesktopEndpoint {
    let trimmed = input.trimmingCharacters(in: .whitespacesAndNewlines)
        .trimmingCharacters(in: CharacterSet(charactersIn: "/"))
    guard !trimmed.isEmpty else { throw LanPulseError.invalidAddress }
    let value = trimmed.hasPrefix("http://") ? trimmed : "http://\(trimmed)"
    guard let components = URLComponents(string: value),
          components.scheme == "http",
          let host = components.host,
          !host.isEmpty,
          let port = components.port,
          (1...65_535).contains(port),
          components.path.isEmpty,
          let url = components.url
    else { throw LanPulseError.invalidAddress }
    return DesktopEndpoint(name: "Manual computer", controlURL: url, audio: nil)
}
