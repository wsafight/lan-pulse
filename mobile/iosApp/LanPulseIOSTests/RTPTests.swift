import XCTest
@testable import LanPulseIOS

final class RTPTests: XCTestCase {
    private let payload = Data(repeating: 0x2A, count: 960)

    func testParsesValidPacket() throws {
        let datagram = makePacket(sequence: 65_535, timestamp: 240, payload: payload)
        let packet = try XCTUnwrap(
            RTPPacket(datagram: datagram, payloadType: 96, ssrc: 0x10203040, payloadBytes: 960)
        )

        XCTAssertEqual(packet.sequence, 65_535)
        XCTAssertEqual(packet.timestamp, 240)
        XCTAssertEqual(packet.payload, payload)
    }

    func testRejectsWrongPayloadTypeAndSSRC() {
        let datagram = makePacket(sequence: 1, timestamp: 0, payload: payload)

        XCTAssertNil(RTPPacket(datagram: datagram, payloadType: 97, ssrc: 0x10203040, payloadBytes: 960))
        XCTAssertNil(RTPPacket(datagram: datagram, payloadType: 96, ssrc: 1, payloadBytes: 960))
    }

    func testJitterBufferOrdersAcrossSequenceWrap() throws {
        var buffer = RTPJitterBuffer(targetPacketCount: 3, maximumPacketCount: 8)
        for sequence in [0, 65_535, 1] as [UInt16] {
            buffer.insert(try XCTUnwrap(RTPPacket(
                datagram: makePacket(sequence: sequence, timestamp: 0, payload: payload),
                payloadType: 96,
                ssrc: 0x10203040,
                payloadBytes: 960
            )))
        }

        buffer.startNearLive()

        XCTAssertEqual(buffer.next()?.sequence, 65_535)
        XCTAssertEqual(buffer.next()?.sequence, 0)
        XCTAssertEqual(buffer.next()?.sequence, 1)
    }

    func testJitterBufferDropsPacketsThatArriveAfterConcealment() throws {
        var buffer = RTPJitterBuffer(targetPacketCount: 2, maximumPacketCount: 8)
        for sequence in [10, 11] as [UInt16] {
            buffer.insert(try parse(makePacket(sequence: sequence, timestamp: 0, payload: payload)))
        }
        buffer.startNearLive()
        XCTAssertEqual(buffer.next()?.sequence, 10)
        XCTAssertEqual(buffer.next()?.sequence, 11)

        buffer.insert(try parse(makePacket(sequence: 10, timestamp: 0, payload: payload)))

        XCTAssertEqual(buffer.count, 0)
    }

    func testJitterBufferTargetCanAdaptWithinMaximum() {
        var buffer = RTPJitterBuffer(targetPacketCount: 3, maximumPacketCount: 8)
        buffer.updateTarget(6)
        XCTAssertEqual(buffer.targetPacketCount, 6)
        buffer.updateTarget(20)
        XCTAssertEqual(buffer.targetPacketCount, 8)
    }

    func testStableNetworkShrinksAdaptiveBuffer() {
        let controller = AdaptiveJitterController(sampleRate: 48_000, packetMs: 5)
        var timestamp: UInt32 = 0
        var arrival: UInt64 = 0
        for _ in 0..<1_200 {
            controller.observe(timestamp: timestamp, arrivalNanos: arrival)
            timestamp &+= 240
            arrival += 5_000_000
        }
        XCTAssertEqual(controller.targetPacketCount, 2)
        XCTAssertLessThan(controller.jitterMs, 0.01)
    }

    func testVariableArrivalsGrowAdaptiveBuffer() {
        let controller = AdaptiveJitterController(sampleRate: 48_000, packetMs: 5)
        var timestamp: UInt32 = 0
        var arrival: UInt64 = 0
        for index in 0..<300 {
            controller.observe(timestamp: timestamp, arrivalNanos: arrival)
            timestamp &+= 240
            arrival += index.isMultiple(of: 2) ? 1_000_000 : 12_000_000
        }
        XCTAssertGreaterThan(controller.targetPacketCount, 3)
        XCTAssertGreaterThan(controller.jitterMs, 1)
    }

    func testRejectsZeroLengthRTPPadding() {
        var datagram = makePacket(sequence: 1, timestamp: 0, payload: payload)
        datagram[0] |= 0x20
        datagram[datagram.count - 1] = 0

        XCTAssertNil(RTPPacket(datagram: datagram, payloadType: 96, ssrc: 0x10203040, payloadBytes: 960))
    }

    func testManualEndpointRequiresHTTPHostAndPort() throws {
        XCTAssertEqual(try manualEndpoint(from: "192.168.1.5:4100").controlURL.absoluteString, "http://192.168.1.5:4100")
        XCTAssertThrowsError(try manualEndpoint(from: "192.168.1.5"))
        XCTAssertThrowsError(try manualEndpoint(from: "https://example.com:4100"))
    }

    private func makePacket(sequence: UInt16, timestamp: UInt32, payload: Data) -> Data {
        var bytes: [UInt8] = [
            0x80,
            96,
            UInt8(sequence >> 8), UInt8(sequence & 0xFF),
            UInt8(timestamp >> 24), UInt8((timestamp >> 16) & 0xFF),
            UInt8((timestamp >> 8) & 0xFF), UInt8(timestamp & 0xFF),
            0x10, 0x20, 0x30, 0x40,
        ]
        bytes.append(contentsOf: payload)
        return Data(bytes)
    }

    private func parse(_ data: Data) throws -> RTPPacket {
        try XCTUnwrap(RTPPacket(
            datagram: data,
            payloadType: 96,
            ssrc: 0x10203040,
            payloadBytes: 960
        ))
    }
}
