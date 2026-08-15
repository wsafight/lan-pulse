package com.lanpulse.mobile.android

import kotlin.math.abs
import kotlin.math.ceil

internal class AdaptiveJitterController(
    private val sampleRate: Int,
    private val packetMs: Int,
) {
    private val minPackets = packetsForDuration(MIN_ADAPTIVE_BUFFER_MS, packetMs)
    private val maxPackets = packetsForDuration(MAX_ADAPTIVE_BUFFER_MS, packetMs)
    private var jitterTicks = 0.0
    private var lastArrivalNanos: Long? = null
    private var lastTimestamp: Long? = null
    private var lastIncreaseNanos: Long? = null
    private var lowerTargetSinceNanos: Long? = null
    private var holdTargetUntilNanos: Long = 0

    var targetPacketCount: Int = packetsForDuration(INITIAL_ADAPTIVE_BUFFER_MS, packetMs)
        private set

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
                adjustTarget(arrivalNanos)
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

    private fun adjustTarget(nowNanos: Long) {
        val guardedJitterMs =
            (jitterMs * JITTER_SAFETY_MULTIPLIER - STABLE_NETWORK_MARGIN_MS).coerceAtLeast(0.0)
        val desiredMs = MIN_ADAPTIVE_BUFFER_MS + ceil(guardedJitterMs).toInt()
        val desiredPackets = packetsForDuration(desiredMs, packetMs).coerceIn(minPackets, maxPackets)
        if (desiredPackets > targetPacketCount) {
            val lastIncrease = lastIncreaseNanos
            if (lastIncrease == null || nowNanos - lastIncrease >= TARGET_GROW_INTERVAL_NANOS) {
                targetPacketCount += 1
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

    private companion object {
        const val INITIAL_ADAPTIVE_BUFFER_MS = 600
        const val MIN_ADAPTIVE_BUFFER_MS = 500
        const val MAX_ADAPTIVE_BUFFER_MS = 800
        const val JITTER_FILTER_DIVISOR = 16.0
        const val JITTER_SAFETY_MULTIPLIER = 4.0
        const val STABLE_NETWORK_MARGIN_MS = 1.0
        const val NANOS_PER_SECOND = 1_000_000_000.0
        const val TARGET_GROW_INTERVAL_NANOS = 250_000_000L
        const val TARGET_SHRINK_STABLE_NANOS = 5_000_000_000L
        const val UNDERRUN_TARGET_HOLD_NANOS = 60_000_000_000L
        const val MAX_UNDERRUN_GROWTH_PACKETS = 8
        const val UINT32_HALF_RANGE = 0x8000_0000L

        fun uint32Distance(from: Long, to: Long): Long = (to - from) and 0xFFFF_FFFFL
    }
}
