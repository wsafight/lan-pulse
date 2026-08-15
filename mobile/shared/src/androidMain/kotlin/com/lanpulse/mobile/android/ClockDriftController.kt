package com.lanpulse.mobile.android

import kotlin.math.abs
import kotlin.math.max

internal class ClockDriftController(private val framesPerPacket: Int) {
    private var correctionCooldownPackets = 0
    private var requestedDirection = 0
    private var consecutiveRequests = 0

    fun correction(queuedFrames: Long, targetFrames: Long): Int {
        val tolerance = max(1, framesPerPacket)
        val errorFrames = queuedFrames - targetFrames
        val requested = when {
            errorFrames > tolerance -> -1
            errorFrames < -tolerance -> 1
            else -> 0
        }
        if (requested == 0) {
            requestedDirection = 0
            consecutiveRequests = 0
            return 0
        }
        if (requested != requestedDirection) {
            requestedDirection = requested
            consecutiveRequests = 1
        } else {
            consecutiveRequests += 1
        }
        val correctionInterval = correctionInterval(abs(errorFrames))
        correctionCooldownPackets = minOf(correctionCooldownPackets, correctionInterval)
        if (correctionCooldownPackets > 0) correctionCooldownPackets -= 1
        if (
            consecutiveRequests < REQUIRED_CONSECUTIVE_REQUESTS ||
            correctionCooldownPackets > 0
        ) {
            return 0
        }

        correctionCooldownPackets = correctionInterval
        return requested
    }

    private fun correctionInterval(errorFrames: Long): Int = when {
        errorFrames > framesPerPacket.toLong() * FAST_CORRECTION_ERROR_PACKETS ->
            FAST_CORRECTION_INTERVAL_PACKETS
        errorFrames > framesPerPacket.toLong() * CATCH_UP_ERROR_PACKETS ->
            CATCH_UP_CORRECTION_INTERVAL_PACKETS
        else -> DRIFT_CORRECTION_INTERVAL_PACKETS
    }

    fun reset() {
        correctionCooldownPackets = 0
        requestedDirection = 0
        consecutiveRequests = 0
    }

    private companion object {
        const val REQUIRED_CONSECUTIVE_REQUESTS = 4
        const val FAST_CORRECTION_ERROR_PACKETS = 4
        const val CATCH_UP_ERROR_PACKETS = 2
        const val FAST_CORRECTION_INTERVAL_PACKETS = 1
        const val CATCH_UP_CORRECTION_INTERVAL_PACKETS = 4
        const val DRIFT_CORRECTION_INTERVAL_PACKETS = 40
    }
}
