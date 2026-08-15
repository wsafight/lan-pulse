package com.lanpulse.mobile.android

import kotlin.math.abs
import kotlin.math.ceil
import kotlin.math.max

internal class AdaptiveJitterController(
    private val sampleRate: Int,
    private val packetMs: Int,
    initialBufferMs: Int = INITIAL_ADAPTIVE_BUFFER_MS,
    minBufferMs: Int = MIN_ADAPTIVE_BUFFER_MS,
    maxBufferMs: Int = MAX_ADAPTIVE_BUFFER_MS,
) {
    private val minPackets = packetsForDuration(minBufferMs, packetMs)
    private val maxPackets = packetsForDuration(maxBufferMs, packetMs)
    private var jitterTicks = 0.0
    private var lastArrivalNanos: Long? = null
    private var lastTimestamp: Long? = null
    private var lastIncreaseNanos: Long? = null
    private var lowerTargetSinceNanos: Long? = null
    private var holdTargetUntilNanos: Long = 0

    var targetPacketCount: Int = packetsForDuration(initialBufferMs, packetMs)
        private set

    init {
        require(minBufferMs > 0)
        require(initialBufferMs in minBufferMs..maxBufferMs)
    }

    val jitterMs: Double
        get() = jitterTicks * 1_000.0 / sampleRate

    fun observe(timestamp: Long, arrivalNanos: Long) {
        val previousArrival = lastArrivalNanos
        val previousTimestamp = lastTimestamp
        if (previousArrival != null && previousTimestamp != null && arrivalNanos > previousArrival) {
            val timestampDelta = uint32Distance(previousTimestamp, timestamp)
            if (timestampDelta in 1..sampleRate.toLong()) {
                val arrivalDeltaTicks =
                    (arrivalNanos - previousArrival).toDouble() * sampleRate / NANOS_PER_SECOND
                val deviation = abs(arrivalDeltaTicks - timestampDelta)
                jitterTicks += (deviation - jitterTicks) / JITTER_FILTER_DIVISOR
                val schedulingDelayMs =
                    ((arrivalDeltaTicks - timestampDelta) * 1_000.0 / sampleRate)
                        .coerceAtLeast(0.0)
                adjustTarget(arrivalNanos, schedulingDelayMs)
                lastArrivalNanos = arrivalNanos
                lastTimestamp = timestamp
            } else if (timestampDelta == 0L || timestampDelta >= UINT32_HALF_RANGE) {
                return
            } else {
                lastArrivalNanos = arrivalNanos
                lastTimestamp = timestamp
            }
        } else {
            lastArrivalNanos = arrivalNanos
            lastTimestamp = timestamp
        }
    }

    fun onUnderrun(nowNanos: Long, severityPackets: Int = 1) {
        val increase = severityPackets.coerceIn(1, MAX_UNDERRUN_GROWTH_PACKETS)
        targetPacketCount = (targetPacketCount + increase).coerceAtMost(maxPackets)
        holdRaisedTarget(nowNanos)
    }

    fun onBacklog(nowNanos: Long) {
        targetPacketCount = maxPackets
        holdRaisedTarget(nowNanos)
    }

    private fun holdRaisedTarget(nowNanos: Long) {
        lastIncreaseNanos = nowNanos
        lowerTargetSinceNanos = null
        holdTargetUntilNanos = maxOf(
            holdTargetUntilNanos,
            nowNanos + UNDERRUN_TARGET_HOLD_NANOS,
        )
    }

    fun resetStreamTiming() {
        jitterTicks = 0.0
        lastArrivalNanos = null
        lastTimestamp = null
        lastIncreaseNanos = null
        lowerTargetSinceNanos = null
        holdTargetUntilNanos = 0
    }

    private fun adjustTarget(nowNanos: Long, schedulingDelayMs: Double) {
        val guardedJitterMs =
            (jitterMs * JITTER_SAFETY_MULTIPLIER - STABLE_NETWORK_MARGIN_MS).coerceAtLeast(0.0)
        val desiredMs = max(
            minPackets * packetMs + ceil(guardedJitterMs).toInt(),
            ceil(schedulingDelayMs).toInt() + SCHEDULING_DELAY_MARGIN_MS,
        )
        val desiredPackets = packetsForDuration(desiredMs, packetMs).coerceIn(minPackets, maxPackets)
        if (desiredPackets > targetPacketCount) {
            val lastIncrease = lastIncreaseNanos
            if (lastIncrease == null || nowNanos - lastIncrease >= TARGET_GROW_INTERVAL_NANOS) {
                targetPacketCount = desiredPackets
                lastIncreaseNanos = nowNanos
            }
            lowerTargetSinceNanos = null
        } else if (desiredPackets < targetPacketCount) {
            if (nowNanos < holdTargetUntilNanos) {
                lowerTargetSinceNanos = null
                return
            }
            val lowerSince = lowerTargetSinceNanos
            if (lowerSince == null) {
                lowerTargetSinceNanos = nowNanos
            } else if (nowNanos - lowerSince >= TARGET_SHRINK_STABLE_NANOS) {
                targetPacketCount -= 1
                lowerTargetSinceNanos = nowNanos
            }
        } else {
            lowerTargetSinceNanos = null
        }
    }

    internal companion object {
        const val INITIAL_ADAPTIVE_BUFFER_MS = 120
        const val MIN_ADAPTIVE_BUFFER_MS = 40
        const val MAX_ADAPTIVE_BUFFER_MS = MAX_BUFFER_MS
        const val JITTER_FILTER_DIVISOR = 16.0
        const val JITTER_SAFETY_MULTIPLIER = 4.0
        const val STABLE_NETWORK_MARGIN_MS = 1.0
        const val SCHEDULING_DELAY_MARGIN_MS = 25
        const val NANOS_PER_SECOND = 1_000_000_000.0
        const val TARGET_GROW_INTERVAL_NANOS = 250_000_000L
        const val TARGET_SHRINK_STABLE_NANOS = 1_000_000_000L
        const val UNDERRUN_TARGET_HOLD_NANOS = 30_000_000_000L
        const val MAX_UNDERRUN_GROWTH_PACKETS = 8
        const val UINT32_HALF_RANGE = 0x8000_0000L

        fun uint32Distance(from: Long, to: Long): Long = (to - from) and 0xFFFF_FFFFL
    }
}
