package com.lanpulse.mobile.android

import com.lanpulse.mobile.PlaybackMode
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class RtpPacketTest {
    @Test
    fun parsesBasicPacketInPlace() {
        val bytes = byteArrayOf(
            0x80.toByte(), 96, 0x12, 0x34, 0, 0, 0, 1, 0, 0, 0, 2,
            1, 2, 3, 4,
        )
        val packet = packetBuffer(bytes)

        assertTrue(packet.parseInPlace(bytes.size, 96, expectedSsrc = 2))
        assertEquals(0x1234, packet.sequence)
        assertEquals(1, packet.timestamp)
        assertEquals(2, packet.ssrc)
        assertContentEquals(byteArrayOf(1, 2, 3, 4), packet.payloadCopy())
    }

    @Test
    fun parsesExtensionAndPaddingInPlace() {
        val bytes = byteArrayOf(
            0xB0.toByte(), 96, 0, 7, 0, 0, 0, 1, 0, 0, 0, 2,
            0xBE.toByte(), 0xDE.toByte(), 0, 1,
            9, 8, 7, 6,
            1, 2, 3, 2, 0, 0, 0, 4,
        )
        val packet = packetBuffer(bytes)

        assertTrue(packet.parseInPlace(bytes.size, 96, expectedSsrc = 2))
        assertEquals(7, packet.sequence)
        assertContentEquals(byteArrayOf(1, 2, 3, 2), packet.payloadCopy())
    }

    @Test
    fun rejectsWrongVersionPayloadTypeAndTruncation() {
        val valid = byteArrayOf(
            0x80.toByte(), 96, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 9,
        )

        assertFalse(packetBuffer(valid.copyOf().also { it[0] = 0x40 }).parseInPlace(valid.size, 96))
        assertFalse(packetBuffer(valid).parseInPlace(valid.size, 97))
        assertFalse(packetBuffer(valid).parseInPlace(valid.size, 96, expectedSsrc = 2))
        assertFalse(packetBuffer(valid).parseInPlace(valid.size + 1, 96))
    }

    @Test
    fun rejectsMalformedRtpExtensionAndPadding() {
        val truncatedExtension = byteArrayOf(
            0x90.toByte(), 96, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1,
            0xBE.toByte(), 0xDE.toByte(), 0, 2,
            9, 8, 7, 6,
        )
        val zeroPadding = byteArrayOf(
            0xA0.toByte(), 96, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1,
            9, 8, 7, 0,
        )
        val tooMuchPadding = byteArrayOf(
            0xA0.toByte(), 96, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1,
            9, 8, 7, 6,
        )

        assertFalse(packetBuffer(truncatedExtension).parseInPlace(truncatedExtension.size, 96))
        assertFalse(packetBuffer(zeroPadding).parseInPlace(zeroPadding.size, 96))
        assertFalse(packetBuffer(tooMuchPadding).parseInPlace(tooMuchPadding.size, 96))
    }

    @Test
    fun jitterBufferWaitsForAReorderedPacketBeforeConcealing() {
        val buffer = RtpJitterBuffer(targetPacketCount = 4, maxBufferedPackets = 12)
        (100..103).forEach { buffer.offer(packet(it)) }
        startAndDrain(buffer, 4)
        assertFalse(buffer.hasPacketAfterExpected())

        buffer.offer(packet(105))
        assertTrue(buffer.hasPacketAfterExpected())
        assertFalse(buffer.shouldConcealExpected())

        buffer.offer(packet(104))
        assertContentEquals(payload(104), buffer.pollExpected()?.payloadCopy())
        assertContentEquals(payload(105), buffer.pollExpected()?.payloadCopy())
        assertFalse(buffer.hasPacketAfterExpected())
    }

    @Test
    fun jitterBufferReportsSequenceSpanForLayeredPlaybackBuffering() {
        val buffer = RtpJitterBuffer(targetPacketCount = 4, maxBufferedPackets = 12)
        (100..103).forEach { buffer.offer(packet(it)) }
        buffer.startNearLive()

        assertEquals(4, buffer.bufferedSpanPackets())
        assertNotNull(buffer.pollExpected())
        assertEquals(3, buffer.bufferedSpanPackets())

        buffer.offer(packet(105))
        assertEquals(5, buffer.bufferedSpanPackets())
    }

    @Test
    fun jitterBufferConcealsOnlyAfterEnoughLookahead() {
        val buffer = RtpJitterBuffer(targetPacketCount = 4, maxBufferedPackets = 12)
        (10..13).forEach { buffer.offer(packet(it)) }
        startAndDrain(buffer, 4)

        buffer.offer(packet(15))
        assertFalse(buffer.shouldConcealExpected())
        buffer.offer(packet(16))
        assertTrue(buffer.shouldConcealExpected())

        buffer.concealExpected()
        assertContentEquals(payload(15), buffer.pollExpected()?.payloadCopy())
    }

    @Test
    fun smallerPacketWindowConcealsAfterOneConfirmedFuturePacket() {
        val buffer = RtpJitterBuffer(targetPacketCount = 2, maxBufferedPackets = 8)
        listOf(20, 21).forEach { buffer.offer(packet(it)) }
        startAndDrain(buffer, 2)

        buffer.offer(packet(23))

        assertTrue(buffer.shouldConcealExpected())
        buffer.concealExpected()
        assertContentEquals(payload(23), buffer.pollExpected()?.payloadCopy())
    }

    @Test
    fun jitterBufferResetsToNewestWindowAfterBacklog() {
        val buffer = RtpJitterBuffer(targetPacketCount = 4, maxBufferedPackets = 12)
        (0..3).forEach { buffer.offer(packet(it)) }
        startAndDrain(buffer, 4)
        (4..41).forEach { buffer.offer(packet(it)) }

        assertTrue(buffer.needsLatencyReset())
        buffer.startNearLive()

        assertContentEquals(payload(38), buffer.pollExpected()?.payloadCopy())
        repeat(2) { assertNotNull(buffer.pollExpected()) }
        assertContentEquals(payload(41), buffer.pollExpected()?.payloadCopy())
        assertEquals(0, buffer.bufferedLeadPackets())
    }

    @Test
    fun jitterBufferReportsWhenModeSpecificLiveLeadIsExceeded() {
        val buffer = RtpJitterBuffer(targetPacketCount = 4, maxBufferedPackets = 12)
        (0..3).forEach { buffer.offer(packet(it)) }
        buffer.startNearLive()
        (4..6).forEach { buffer.offer(packet(it)) }

        assertFalse(buffer.exceedsLeadLimit(6))
        buffer.offer(packet(7))
        assertTrue(buffer.exceedsLeadLimit(6))
    }

    @Test
    fun jitterBufferKeepsRecoverableBacklogAfterReceiverSchedulingPause() {
        val buffer = RtpJitterBuffer(targetPacketCount = 4, maxBufferedPackets = 12)
        (100..103).forEach { buffer.offer(packet(it)) }
        startAndDrain(buffer, 4)

        (104..140).forEach { buffer.offer(packet(it)) }

        assertFalse(buffer.needsLatencyReset())
        assertContentEquals(payload(104), buffer.pollExpected()?.payloadCopy())
    }

    @Test
    fun jitterBufferHandlesSequenceWrap() {
        val buffer = RtpJitterBuffer(targetPacketCount = 4, maxBufferedPackets = 12)
        listOf(65_534, 65_535, 0, 1).forEach { buffer.offer(packet(it)) }

        buffer.startNearLive()

        assertContentEquals(payload(65_534), buffer.pollExpected()?.payloadCopy())
        repeat(2) { assertNotNull(buffer.pollExpected()) }
        assertContentEquals(payload(1), buffer.pollExpected()?.payloadCopy())
        buffer.offer(packet(2))
        assertContentEquals(payload(2), buffer.pollExpected()?.payloadCopy())
    }

    @Test
    fun jitterBufferCanResetAfterSenderSequenceRestarts() {
        val buffer = RtpJitterBuffer(targetPacketCount = 4, maxBufferedPackets = 12)
        (100..103).forEach { buffer.offer(packet(it)) }
        startAndDrain(buffer, 4)
        (0..3).forEach { buffer.offer(packet(it)) }

        assertTrue(buffer.needsSequenceReset())
        buffer.reset()
        (4..7).forEach { buffer.offer(packet(it)) }

        buffer.startNearLive()
        assertContentEquals(payload(4), buffer.pollExpected()?.payloadCopy())
        repeat(2) { assertNotNull(buffer.pollExpected()) }
        assertContentEquals(payload(7), buffer.pollExpected()?.payloadCopy())
    }

    @Test
    fun jitterBufferDoesNotResetForConsecutivePacketsFromItsNormalHistoryWindow() {
        val buffer = RtpJitterBuffer(targetPacketCount = 4, maxBufferedPackets = 12)
        (196..199).forEach { buffer.offer(packet(it)) }
        startAndDrain(buffer, 4)

        (152..159).forEach { buffer.offer(packet(it)) }

        assertFalse(buffer.needsSequenceReset())
        assertEquals(8, buffer.latePackets)
    }

    @Test
    fun jitterBufferRecyclesDuplicateAndReplacedPackets() {
        val recycled = mutableListOf<Int>()
        val buffer = RtpJitterBuffer(
            targetPacketCount = 2,
            maxBufferedPackets = 4,
            recycle = { recycled += it.sequence },
        )
        buffer.offer(packet(1))
        buffer.offer(packet(1))
        buffer.offer(packet(17))

        assertEquals(listOf(1, 1), recycled)
        assertEquals(1, buffer.duplicatePackets)
        assertEquals(1, buffer.replacedPackets)
    }

    @Test
    fun jitterBufferUpdateTargetIsClampedToConfiguredWindow() {
        val buffer = RtpJitterBuffer(targetPacketCount = 2, maxBufferedPackets = 4)

        buffer.updateTarget(0)
        assertEquals(1, buffer.targetPacketCount)
        buffer.updateTarget(99)
        assertEquals(4, buffer.targetPacketCount)
    }

    @Test
    fun jitterBufferPrunesOldSlotsIncrementally() {
        val recycled = mutableListOf<Int>()
        val buffer = RtpJitterBuffer(
            targetPacketCount = 2,
            maxBufferedPackets = 4,
            recycle = { recycled += it.sequence },
        )
        buffer.offer(packet(0))

        (20..23).forEach { buffer.offer(packet(it)) }

        assertEquals(1, buffer.prunedPackets)
        assertEquals(listOf(0), recycled)
    }

    @Test
    fun encodesBoundedVersionedRtpNack() {
        val bytes = ByteArray(RTP_NACK_PACKET_BYTES)

        encodeRtpNack(bytes, sequence = 0x1234, ssrc = 0x5566_7788)

        assertContentEquals(
            byteArrayOf(
                'L'.code.toByte(),
                'P'.code.toByte(),
                'N'.code.toByte(),
                'K'.code.toByte(),
                1,
                0,
                0x12,
                0x34,
                0x55,
                0x66,
                0x77,
                0x88.toByte(),
            ),
            bytes,
        )
    }

    @Test
    fun stableNetworkShrinksAdaptiveBufferToFortyMilliseconds() {
        val controller = AdaptiveJitterController(sampleRate = 48_000, packetMs = 5)
        var timestamp = 0L
        var arrival = 0L

        repeat(24_000) {
            controller.observe(timestamp, arrival)
            timestamp = (timestamp + 240) and 0xFFFF_FFFFL
            arrival += 5_000_000
        }

        assertEquals(8, controller.targetPacketCount)
        assertTrue(controller.jitterMs < 0.01)
    }

    @Test
    fun duplicatePacketDoesNotPerturbJitterEstimate() {
        val controller = AdaptiveJitterController(sampleRate = 48_000, packetMs = 5)

        controller.observe(timestamp = 0, arrivalNanos = 0)
        controller.observe(timestamp = 240, arrivalNanos = 5_000_000)
        controller.observe(timestamp = 240, arrivalNanos = 6_000_000)
        controller.observe(timestamp = 480, arrivalNanos = 10_000_000)

        assertEquals(0.0, controller.jitterMs)
    }

    @Test
    fun variableArrivalTimesGrowAdaptiveBuffer() {
        val controller = AdaptiveJitterController(sampleRate = 48_000, packetMs = 5)
        var timestamp = 0L
        var arrival = 0L

        repeat(300) { index ->
            controller.observe(timestamp, arrival)
            timestamp = (timestamp + 240) and 0xFFFF_FFFFL
            arrival += if (index % 2 == 0) 1_000_000 else 100_000_000
        }

        assertTrue(controller.targetPacketCount > 24)
        assertTrue(controller.jitterMs > 1.0)
    }

    @Test
    fun adaptiveBufferReturnsToFortyMillisecondsAfterNetworkStabilizes() {
        val controller = AdaptiveJitterController(sampleRate = 48_000, packetMs = 5)
        var timestamp = 0L
        var arrival = 1_000_000_000L
        controller.onBacklog(arrival)

        repeat(75_000) {
            controller.observe(timestamp, arrival)
            timestamp = (timestamp + 240) and 0xFFFF_FFFFL
            arrival += 5_000_000
        }

        assertEquals(8, controller.targetPacketCount)
    }

    @Test
    fun underrunImmediatelyRaisesAdaptiveTarget() {
        val controller = AdaptiveJitterController(sampleRate = 48_000, packetMs = 5)

        controller.onUnderrun(1_000_000_000)

        assertEquals(25, controller.targetPacketCount)
    }

    @Test
    fun batchedUnderrunsQuicklyRaiseAndHoldAdaptiveTarget() {
        val controller = AdaptiveJitterController(sampleRate = 48_000, packetMs = 5)
        val underrunAt = 1_000_000_000L
        controller.onUnderrun(underrunAt, severityPackets = 8)
        var timestamp = 0L
        var arrival = underrunAt

        repeat(4_000) {
            controller.observe(timestamp, arrival)
            timestamp = (timestamp + 240) and 0xFFFF_FFFFL
            arrival += 5_000_000
        }

        assertEquals(32, controller.targetPacketCount)
    }

    @Test
    fun receiverSchedulingBacklogRaisesAdaptiveTargetToMaximum() {
        val controller = AdaptiveJitterController(sampleRate = 48_000, packetMs = 5)

        controller.onBacklog(1_000_000_000L)

        assertEquals(90, controller.targetPacketCount)
    }

    @Test
    fun schedulingDelaySpikeRaisesAdaptiveTargetImmediately() {
        val controller = AdaptiveJitterController(sampleRate = 48_000, packetMs = 5)

        controller.observe(timestamp = 0, arrivalNanos = 0)
        controller.observe(timestamp = 240, arrivalNanos = 300_000_000)

        assertEquals(64, controller.targetPacketCount)
    }

    @Test
    fun immediatePlaybackUsesOnePacketAndDirectWrites() {
        val policy = playbackBufferPolicy(PlaybackMode.Immediate)
        val controller = AdaptiveJitterController(
            sampleRate = 48_000,
            packetMs = 5,
            initialBufferMs = policy.initialBufferMs,
            minBufferMs = policy.minBufferMs,
            maxBufferMs = policy.maxBufferMs,
        )

        controller.observe(timestamp = 0, arrivalNanos = 0)
        controller.observe(timestamp = 240, arrivalNanos = 300_000_000)
        controller.onUnderrun(300_000_000, severityPackets = 8)
        controller.onBacklog(300_000_000)

        assertEquals(1, controller.targetPacketCount)
        assertEquals(1, policy.outputBufferMs)
        assertTrue(policy.directWrite)
    }

    @Test
    fun adaptivePlaybackUsesAutomaticFortyToFourHundredFiftyMillisecondRange() {
        val policy = playbackBufferPolicy(PlaybackMode.Adaptive)

        assertEquals(40, policy.minBufferMs)
        assertEquals(120, policy.initialBufferMs)
        assertEquals(450, policy.maxBufferMs)
        assertEquals(60, policy.outputBufferMs)
        assertFalse(policy.directWrite)
    }

    @Test
    fun playbackClockHandlesUint32Wrap() {
        val clock = AudioPlaybackClock()
        clock.reset(0xFFFF_FFF0.toInt())
        clock.recordWritten(64)

        assertEquals(32, clock.queuedFrames(0x0000_0010))
    }

    @Test
    fun driftControllerRequiresPersistentErrorAndRateLimitsCorrections() {
        val controller = ClockDriftController(framesPerPacket = 240)

        assertEquals(listOf(0, 0, 0, -1), List(4) { controller.correction(1_200, 720) })
        assertTrue(List(39) { controller.correction(1_200, 720) }.all { it == 0 })
        assertEquals(-1, controller.correction(1_200, 720))
        controller.reset()
        assertEquals(listOf(0, 0, 0, 1), List(4) { controller.correction(200, 720) })
        assertEquals(0, controller.correction(700, 720))
    }

    @Test
    fun driftControllerTemporarilyAcceleratesForLargeQueueError() {
        val controller = ClockDriftController(framesPerPacket = 240)
        repeat(4) { controller.correction(1_200, 720) }

        assertEquals(-1, controller.correction(2_000, 720))
        assertEquals(-1, controller.correction(2_000, 720))
    }

    @Test
    fun missingPacketWaitUsesAvailablePlaybackQueueWithoutDrainingItsReserve() {
        assertEquals(5, missingPacketWaitMs(packetMs = 5, queuedFrames = 240, sampleRate = 48_000))
        assertEquals(5, missingPacketWaitMs(packetMs = 5, queuedFrames = 1_440, sampleRate = 48_000))
        assertEquals(22, missingPacketWaitMs(packetMs = 5, queuedFrames = 3_936, sampleRate = 48_000))
        assertEquals(40, missingPacketWaitMs(packetMs = 5, queuedFrames = 4_800, sampleRate = 48_000))
        assertEquals(100, missingPacketWaitMs(packetMs = 5, queuedFrames = 9_600, sampleRate = 48_000))
    }

    @Test
    fun missingPacketCanWaitOnlyWhilePlaybackQueueExceedsItsSafetyReserve() {
        assertFalse(canWaitForMissingPacket(queuedFrames = 2_880, sampleRate = 48_000))
        assertTrue(canWaitForMissingPacket(queuedFrames = 2_881, sampleRate = 48_000))
        assertTrue(canWaitForMissingPacket(queuedFrames = 9_600, sampleRate = 48_000))
    }

    @Test
    fun pcmAdjusterChangesExactlyOneStereoFrame() {
        val input = ByteArray(16) { it.toByte() }
        val adjuster = Pcm16FrameAdjuster(maxPayloadBytes = input.size, bytesPerFrame = 4)

        val shortened = adjuster.adjust(input, 0, input.size, correction = -1)
        val lengthened = adjuster.adjust(input, 0, input.size, correction = 1)

        assertEquals(12, shortened)
        assertEquals(20, lengthened)
    }

    @Test
    fun pcmAdjusterSmoothsTheSampleSlipAcrossMultipleFrames() {
        val input = ByteArray(64 * 2)
        repeat(64) { frame ->
            val sample = frame * 100
            input[frame * 2] = sample.toByte()
            input[frame * 2 + 1] = (sample shr 8).toByte()
        }
        val adjuster = Pcm16FrameAdjuster(maxPayloadBytes = input.size, bytesPerFrame = 2)

        val shortened = adjuster.adjust(input, 0, input.size, correction = -1)
        val shortenedSamples = samples(adjuster.output, shortened)
        val lengthened = adjuster.adjust(input, 0, input.size, correction = 1)
        val lengthenedSamples = samples(adjuster.output, lengthened)

        assertTrue(shortenedSamples.zipWithNext().all { (first, second) -> second - first in 0..110 })
        assertTrue(lengthenedSamples.zipWithNext().all { (first, second) -> second - first in 0..110 })
    }

    @Test
    fun spscRingReusesSlotsAcrossWrap() {
        val ring = SpscRing<Int>(4)
        repeat(4) { assertTrue(ring.offer(it)) }
        assertFalse(ring.offer(4))
        assertEquals(0, ring.poll())
        assertEquals(1, ring.poll())
        assertTrue(ring.offer(4))
        assertTrue(ring.offer(5))

        assertEquals(listOf(2, 3, 4, 5), List(4) { ring.poll() })
        assertNull(ring.poll())
    }

    private fun startAndDrain(buffer: RtpJitterBuffer, packets: Int) {
        buffer.startNearLive()
        repeat(packets) { assertNotNull(buffer.pollExpected()) }
    }

    private fun packetBuffer(bytes: ByteArray): RtpPacketBuffer =
        RtpPacketBuffer(bytes.size).also { bytes.copyInto(it.bytes) }

    private fun packet(sequence: Int): RtpPacketBuffer = RtpPacketBuffer(1).apply {
        this.sequence = sequence
        timestamp = sequence.toLong()
        payloadOffset = 0
        payloadLength = 1
        bytes[0] = (sequence and 0xFF).toByte()
    }

    private fun payload(sequence: Int): ByteArray = byteArrayOf((sequence and 0xFF).toByte())

    private fun samples(bytes: ByteArray, length: Int): List<Int> =
        (0 until length step 2).map { offset ->
            (((bytes[offset + 1].toInt() and 0xFF) shl 8) or (bytes[offset].toInt() and 0xFF))
                .toShort()
                .toInt()
        }
}
