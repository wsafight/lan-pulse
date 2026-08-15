package com.lanpulse.mobile.android

import kotlin.math.max

internal class ClockDriftController(private val framesPerPacket: Int) {
    private var correctionCooldownPackets = 0
    private var requestedDirection = 0
    private var consecutiveRequests = 0

    fun correction(queuedFrames: Long, targetFrames: Long): Int {
        if (correctionCooldownPackets > 0) correctionCooldownPackets -= 1
        val tolerance = max(1, framesPerPacket)
        val requested = when {
            queuedFrames > targetFrames + tolerance -> -1
            queuedFrames + tolerance < targetFrames -> 1
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
        if (
            consecutiveRequests < REQUIRED_CONSECUTIVE_REQUESTS ||
            correctionCooldownPackets > 0
        ) {
            return 0
        }

        correctionCooldownPackets = CORRECTION_INTERVAL_PACKETS
        return requested
    }

    fun reset() {
        correctionCooldownPackets = 0
        requestedDirection = 0
        consecutiveRequests = 0
    }

    private companion object {
        const val REQUIRED_CONSECUTIVE_REQUESTS = 4
        const val CORRECTION_INTERVAL_PACKETS = 40
    }
}
