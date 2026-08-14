package com.lanpulse.mobile.android

import kotlin.math.max

internal class ClockDriftController(private val framesPerPacket: Int) {
    fun correction(queuedFrames: Long, targetFrames: Long): Int {
        val tolerance = max(1, framesPerPacket / 2)
        return when {
            queuedFrames > targetFrames + tolerance -> -1
            queuedFrames + tolerance < targetFrames -> 1
            else -> 0
        }
    }
}
