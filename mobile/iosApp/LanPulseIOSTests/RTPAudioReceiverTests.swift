import Darwin
import XCTest
@testable import LanPulseIOS

final class RTPAudioReceiverTests: XCTestCase {
    func testReceivesRTPAndStartsAudioPlayout() throws {
        let audio = AudioConfig(
            sampleRate: 48_000,
            channels: 2,
            sampleFormat: "s16le",
            packetMs: 5,
            payloadType: 96,
            ssrc: 0x10203040
        )
        let receiver = try RTPAudioReceiver()
        let played = expectation(description: "RTP audio reached playout")
        try receiver.start(
            audio: audio,
            onStats: { stats in
                if stats.packetsReceived > 0 { played.fulfill() }
            },
            onFailure: { error in
                XCTFail("Unexpected receiver failure: \(error)")
            }
        )
        defer { receiver.stop() }

        let descriptor = Darwin.socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP)
        XCTAssertGreaterThanOrEqual(descriptor, 0)
        defer { Darwin.close(descriptor) }
        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_port = receiver.localPort.bigEndian
        address.sin_addr = in_addr(s_addr: inet_addr("127.0.0.1"))

        for sequence in UInt16(0)..<12 {
            let packet = makePacket(sequence: sequence, audio: audio)
            packet.withUnsafeBytes { bytes in
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

        wait(for: [played], timeout: 2)
    }

    private func makePacket(sequence: UInt16, audio: AudioConfig) -> Data {
        let timestamp = UInt32(sequence) * UInt32(audio.framesPerPacket)
        var bytes: [UInt8] = [
            0x80,
            audio.payloadType,
            UInt8(sequence >> 8), UInt8(sequence & 0xFF),
            UInt8(timestamp >> 24), UInt8((timestamp >> 16) & 0xFF),
            UInt8((timestamp >> 8) & 0xFF), UInt8(timestamp & 0xFF),
            0x10, 0x20, 0x30, 0x40,
        ]
        bytes.append(contentsOf: repeatElement(0, count: audio.payloadBytes))
        return Data(bytes)
    }
}
