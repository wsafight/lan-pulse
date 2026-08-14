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

    fun onUnderrun(nowNanos: Long) {
        targetPacketCount = (targetPacketCount + 1).coerceAtMost(maxPackets)
        lastIncreaseNanos = nowNanos
        lowerTargetSinceNanos = null
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
        const val INITIAL_ADAPTIVE_BUFFER_MS = 15
        const val MIN_ADAPTIVE_BUFFER_MS = 10
        const val MAX_ADAPTIVE_BUFFER_MS = 60
        const val JITTER_FILTER_DIVISOR = 16.0
        const val JITTER_SAFETY_MULTIPLIER = 4.0
        const val STABLE_NETWORK_MARGIN_MS = 1.0
        const val NANOS_PER_SECOND = 1_000_000_000.0
        const val TARGET_GROW_INTERVAL_NANOS = 250_000_000L
        const val TARGET_SHRINK_STABLE_NANOS = 5_000_000_000L
        const val UINT32_HALF_RANGE = 0x8000_0000L

        fun uint32Distance(from: Long, to: Long): Long = (to - from) and 0xFFFF_FFFFL
    }
}
