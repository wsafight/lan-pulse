import Foundation

final class RTPPacket: @unchecked Sendable, Equatable {
    private var bytes: [UInt8]
    private(set) var sequence: UInt16 = 0
    private(set) var timestamp: UInt32 = 0
    private var payloadOffset = 0
    private(set) var payloadLength = 0

    var payload: Data {
        Data(bytes[payloadOffset..<(payloadOffset + payloadLength)])
    }

    init(capacity: Int) {
        bytes = [UInt8](repeating: 0, count: capacity)
    }

    convenience init?(datagram: Data, payloadType: UInt8, ssrc: UInt32, payloadBytes: Int) {
        self.init(capacity: datagram.count)
        let copied = bytes.withUnsafeMutableBytes { destination in
            datagram.copyBytes(to: destination)
        }
        guard parse(length: copied, payloadType: payloadType, ssrc: ssrc, payloadBytes: payloadBytes) else {
            return nil
        }
    }

    func parse(length: Int, payloadType: UInt8, ssrc: UInt32, payloadBytes: Int) -> Bool {
        payloadLength = 0
        guard length >= 12, length <= bytes.count else { return false }
        guard bytes[0] >> 6 == 2 else { return false }
        guard bytes[1] & 0x7F == payloadType else { return false }

        let hasPadding = bytes[0] & 0x20 != 0
        let hasExtension = bytes[0] & 0x10 != 0
        let contributingSources = Int(bytes[0] & 0x0F)
        var payloadOffset = 12 + contributingSources * 4
        guard payloadOffset <= length else { return false }

        if hasExtension {
            guard payloadOffset + 4 <= length else { return false }
            let words = Int(readUInt16(bytes, at: payloadOffset + 2))
            payloadOffset += 4 + words * 4
            guard payloadOffset <= length else { return false }
        }

        let paddingBytes = hasPadding ? Int(bytes[length - 1]) : 0
        guard !hasPadding || paddingBytes > 0 else { return false }
        guard paddingBytes <= length - payloadOffset else { return false }
        let payloadEnd = length - paddingBytes
        guard payloadEnd - payloadOffset == payloadBytes else { return false }
        guard readUInt32(bytes, at: 8) == ssrc else { return false }

        sequence = readUInt16(bytes, at: 2)
        timestamp = readUInt32(bytes, at: 4)
        self.payloadOffset = payloadOffset
        payloadLength = payloadEnd - payloadOffset
        return true
    }

    func withUnsafeMutableDatagramBytes<Result>(
        _ body: (UnsafeMutableRawBufferPointer) throws -> Result
    ) rethrows -> Result {
        try bytes.withUnsafeMutableBytes(body)
    }

    func copyPayload(to destination: UnsafeMutableRawPointer, capacity: Int) {
        bytes.withUnsafeBytes { source in
            guard let baseAddress = source.baseAddress else { return }
            memcpy(destination, baseAddress.advanced(by: payloadOffset), min(payloadLength, capacity))
        }
    }

    func withUnsafePayloadBytes<Result>(
        _ body: (UnsafeRawBufferPointer) throws -> Result
    ) rethrows -> Result {
        try bytes.withUnsafeBytes { source in
            let payload = UnsafeRawBufferPointer(rebasing: source[payloadOffset..<(payloadOffset + payloadLength)])
            return try body(payload)
        }
    }

    static func == (lhs: RTPPacket, rhs: RTPPacket) -> Bool {
        lhs.sequence == rhs.sequence && lhs.timestamp == rhs.timestamp && lhs.payload == rhs.payload
    }
}

struct RTPJitterBuffer: Sendable {
    private(set) var expectedSequence: UInt16?
    private var packets: [UInt16: RTPPacket] = [:]
    private var recycledPackets: [RTPPacket] = []
    private(set) var targetPacketCount: Int
    let maximumPacketCount: Int

    init(targetPacketCount: Int = 3, maximumPacketCount: Int = 12) {
        self.targetPacketCount = targetPacketCount
        self.maximumPacketCount = maximumPacketCount
    }

    var isReady: Bool { packets.count >= targetPacketCount }
    var count: Int { packets.count }

    mutating func updateTarget(_ packetCount: Int) {
        targetPacketCount = min(max(1, packetCount), maximumPacketCount)
    }

    mutating func insert(_ packet: RTPPacket) {
        if let expectedSequence, sequenceIsOlder(packet.sequence, than: expectedSequence) {
            recycledPackets.append(packet)
            return
        }
        guard packets[packet.sequence] == nil else {
            recycledPackets.append(packet)
            return
        }
        packets[packet.sequence] = packet
        if packets.count > maximumPacketCount {
            startNearLive()
        }
    }

    mutating func startNearLive() {
        guard let newest = packets.keys.max(by: { sequenceIsOlder($0, than: $1) }) else { return }
        expectedSequence = newest &- UInt16(clamping: targetPacketCount - 1)
        let staleSequences = packets.keys.filter { sequenceIsOlder($0, than: expectedSequence!) }
        for sequence in staleSequences {
            if let packet = packets.removeValue(forKey: sequence) {
                recycledPackets.append(packet)
            }
        }
    }

    mutating func next() -> RTPPacket? {
        guard let expectedSequence else { return nil }
        let packet = packets.removeValue(forKey: expectedSequence)
        self.expectedSequence = expectedSequence &+ 1
        return packet
    }

    mutating func reset() {
        recycledPackets.append(contentsOf: packets.values)
        packets.removeAll(keepingCapacity: true)
        expectedSequence = nil
    }

    mutating func takeRecycledPacket() -> RTPPacket? {
        recycledPackets.popLast()
    }
}

final class AdaptiveJitterController {
    private let sampleRate: Int
    private let packetMs: Int
    private let minimumPackets: Int
    private let maximumPackets: Int
    private var jitterTicks = 0.0
    private var lastArrivalNanos: UInt64?
    private var lastTimestamp: UInt32?
    private var lastIncreaseNanos: UInt64?
    private var lowerTargetSinceNanos: UInt64?

    private(set) var targetPacketCount: Int
    var jitterMs: Double { jitterTicks * 1_000 / Double(sampleRate) }

    init(sampleRate: Int, packetMs: Int) {
        self.sampleRate = sampleRate
        self.packetMs = packetMs
        minimumPackets = Self.packets(for: 10, packetMs: packetMs)
        maximumPackets = Self.packets(for: 60, packetMs: packetMs)
        targetPacketCount = Self.packets(for: 15, packetMs: packetMs)
    }

    func observe(timestamp: UInt32, arrivalNanos: UInt64) {
        guard let previousArrival = lastArrivalNanos,
              let previousTimestamp = lastTimestamp,
              arrivalNanos > previousArrival
        else {
            lastArrivalNanos = arrivalNanos
            lastTimestamp = timestamp
            return
        }

        let timestampDelta = UInt64(timestamp &- previousTimestamp)
        if timestampDelta == 0 || timestampDelta >= 0x8000_0000 {
            return
        }
        if timestampDelta <= UInt64(sampleRate) {
            let arrivalDeltaTicks = Double(arrivalNanos - previousArrival)
                * Double(sampleRate) / 1_000_000_000
            let deviation = abs(arrivalDeltaTicks - Double(timestampDelta))
            jitterTicks += (deviation - jitterTicks) / 16
            adjustTarget(nowNanos: arrivalNanos)
        }
        lastArrivalNanos = arrivalNanos
        lastTimestamp = timestamp
    }

    func onUnderrun(nowNanos: UInt64) {
        if let lastIncreaseNanos, nowNanos - lastIncreaseNanos < 250_000_000 {
            return
        }
        targetPacketCount = min(targetPacketCount + 1, maximumPackets)
        lastIncreaseNanos = nowNanos
        lowerTargetSinceNanos = nil
    }

    private func adjustTarget(nowNanos: UInt64) {
        let guardedJitterMs = max(0, jitterMs * 4 - 1)
        let desiredMs = 10 + Int(ceil(guardedJitterMs))
        let desiredPackets = min(
            max(Self.packets(for: desiredMs, packetMs: packetMs), minimumPackets),
            maximumPackets
        )
        if desiredPackets > targetPacketCount {
            if lastIncreaseNanos == nil || nowNanos - lastIncreaseNanos! >= 250_000_000 {
                targetPacketCount += 1
                lastIncreaseNanos = nowNanos
            }
            lowerTargetSinceNanos = nil
        } else if desiredPackets < targetPacketCount {
            if let lowerTargetSinceNanos {
                if nowNanos - lowerTargetSinceNanos >= 5_000_000_000 {
                    targetPacketCount -= 1
                    self.lowerTargetSinceNanos = nowNanos
                }
            } else {
                lowerTargetSinceNanos = nowNanos
            }
        } else {
            lowerTargetSinceNanos = nil
        }
    }

    private static func packets(for durationMs: Int, packetMs: Int) -> Int {
        max(1, (durationMs + packetMs - 1) / packetMs)
    }
}

private func readUInt16(_ bytes: [UInt8], at offset: Int) -> UInt16 {
    UInt16(bytes[offset]) << 8 | UInt16(bytes[offset + 1])
}

private func readUInt32(_ bytes: [UInt8], at offset: Int) -> UInt32 {
    UInt32(bytes[offset]) << 24 |
        UInt32(bytes[offset + 1]) << 16 |
        UInt32(bytes[offset + 2]) << 8 |
        UInt32(bytes[offset + 3])
}

private func sequenceIsOlder(_ lhs: UInt16, than rhs: UInt16) -> Bool {
    Int16(bitPattern: lhs &- rhs) < 0
}
