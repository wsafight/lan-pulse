package com.lanpulse.mobile.android

import kotlin.math.max

internal class RtpJitterBuffer(
    targetPacketCount: Int,
    private val maxBufferedPackets: Int,
    private val recycle: (RtpPacketBuffer) -> Unit = {},
) {
    private val slots = arrayOfNulls<RtpPacketBuffer>(
        nextPowerOfTwo(maxBufferedPackets * STORED_PACKET_MULTIPLIER),
    )
    private val slotMask = slots.size - 1
    private var pendingCount = 0
    private var expectedSequence: Int? = null
    private var latestSequence: Int? = null
    private var sequenceRestartCandidate: Int? = null
    private var sequenceRestartCandidatePackets = 0

    var duplicatePackets: Long = 0
        private set
    var latePackets: Long = 0
        private set
    var replacedPackets: Long = 0
        private set
    var prunedPackets: Long = 0
        private set

    var targetPacketCount: Int = targetPacketCount
        private set

    init {
        require(targetPacketCount > 0)
        require(maxBufferedPackets >= targetPacketCount)
    }

    fun updateTarget(packets: Int) {
        targetPacketCount = packets.coerceIn(1, maxBufferedPackets)
    }

    fun offer(packet: RtpPacketBuffer) {
        val expected = expectedSequence
        if (expected != null && forwardDistance(expected, packet.sequence) >= HALF_SEQUENCE_RANGE) {
            latePackets += 1
            observeSequenceRestartCandidate(packet.sequence, expected)
            recycle(packet)
            return
        }
        clearSequenceRestartCandidate()

        val latest = latestSequence
        if (latest == null || isNewer(packet.sequence, latest)) {
            latestSequence = packet.sequence
        }
        val slot = packet.sequence and slotMask
        val existing = slots[slot]
        if (existing != null) {
            if (existing.sequence == packet.sequence) {
                duplicatePackets += 1
                recycle(packet)
                return
            }
            slots[slot] = null
            pendingCount -= 1
            replacedPackets += 1
            recycle(existing)
        }
        slots[slot] = packet
        pendingCount += 1
        pruneOldPackets()
    }

    fun readyToStart(): Boolean = pendingCount >= targetPacketCount

    fun startNearLive() {
        val latest = checkNotNull(latestSequence)
        check(readyToStart())
        expectedSequence = (latest - targetPacketCount + 1) and SEQUENCE_MASK
        clearSequenceRestartCandidate()
        pruneBeforeExpected()
    }

    fun pollExpected(): RtpPacketBuffer? {
        val expected = expectedSequence ?: return null
        val slot = expected and slotMask
        val packet = slots[slot]
        if (packet == null || packet.sequence != expected) return null
        slots[slot] = null
        pendingCount -= 1
        expectedSequence = nextSequence(expected)
        pruneBeforeExpected()
        return packet
    }

    fun concealExpected() {
        val expected = checkNotNull(expectedSequence)
        val slot = expected and slotMask
        val packet = slots[slot]
        if (packet != null && packet.sequence == expected) {
            slots[slot] = null
            pendingCount -= 1
            recycle(packet)
        }
        expectedSequence = nextSequence(expected)
        pruneBeforeExpected()
    }

    fun shouldConcealExpected(): Boolean {
        val expected = expectedSequence ?: return false
        val packet = slots[expected and slotMask]
        if (packet != null && packet.sequence == expected) return false
        return bufferedLeadPackets() >= missingPacketLookahead()
    }

    fun needsLatencyReset(): Boolean =
        bufferedLeadPackets() > maxBufferedPackets * LATENCY_RESET_MULTIPLIER

    fun needsSequenceReset(): Boolean =
        sequenceRestartCandidatePackets >= LATE_PACKET_RESET_THRESHOLD

    fun reset() {
        slots.indices.forEach { slot ->
            slots[slot]?.let(recycle)
            slots[slot] = null
        }
        pendingCount = 0
        expectedSequence = null
        latestSequence = null
        clearSequenceRestartCandidate()
    }

    internal fun bufferedLeadPackets(): Int {
        val expected = expectedSequence ?: return 0
        val latest = latestSequence ?: return 0
        val distance = forwardDistance(expected, latest)
        return if (distance < HALF_SEQUENCE_RANGE) distance else 0
    }

    private fun pruneBeforeExpected() {
        val expected = expectedSequence ?: return
        slots.indices.forEach { slot ->
            val packet = slots[slot] ?: return@forEach
            if (forwardDistance(expected, packet.sequence) >= HALF_SEQUENCE_RANGE) {
                slots[slot] = null
                pendingCount -= 1
                prunedPackets += 1
                recycle(packet)
            }
        }
    }

    private fun pruneOldPackets() {
        val latest = latestSequence ?: return
        val maximumAge = maxBufferedPackets * STORED_PACKET_MULTIPLIER
        slots.indices.forEach { slot ->
            val packet = slots[slot] ?: return@forEach
            val age = forwardDistance(packet.sequence, latest)
            if (age < HALF_SEQUENCE_RANGE && age > maximumAge) {
                slots[slot] = null
                pendingCount -= 1
                prunedPackets += 1
                recycle(packet)
            }
        }
    }

    private fun observeSequenceRestartCandidate(sequence: Int, expected: Int) {
        val packetsBehind = forwardDistance(sequence, expected)
        val normalHistoryPackets = maxBufferedPackets * STORED_PACKET_MULTIPLIER
        if (packetsBehind <= normalHistoryPackets) {
            clearSequenceRestartCandidate()
            return
        }

        val previousCandidate = sequenceRestartCandidate
        val advancesCandidate = previousCandidate != null &&
            forwardDistance(previousCandidate, sequence) in 1..MAX_RESTART_CANDIDATE_GAP
        sequenceRestartCandidatePackets = if (advancesCandidate) {
            sequenceRestartCandidatePackets + 1
        } else {
            1
        }
        sequenceRestartCandidate = sequence
    }

    private fun clearSequenceRestartCandidate() {
        sequenceRestartCandidate = null
        sequenceRestartCandidatePackets = 0
    }

    private fun missingPacketLookahead(): Int = max(1, targetPacketCount / 2)

    internal companion object {
        const val SEQUENCE_MASK = 0xFFFF
        const val HALF_SEQUENCE_RANGE = 0x8000
        const val LATE_PACKET_RESET_THRESHOLD = 4
        const val MAX_RESTART_CANDIDATE_GAP = 4
        const val LATENCY_RESET_MULTIPLIER = 3
        const val STORED_PACKET_MULTIPLIER = 4

        fun nextSequence(sequence: Int): Int = (sequence + 1) and SEQUENCE_MASK

        fun forwardDistance(from: Int, to: Int): Int = (to - from) and SEQUENCE_MASK

        fun isNewer(sequence: Int, reference: Int): Boolean {
            val distance = forwardDistance(reference, sequence)
            return distance in 1 until HALF_SEQUENCE_RANGE
        }
    }
}
