package com.lanpulse.mobile.android

internal class AudioPlaybackClock {
    private var basePlaybackFrame = 0L
    private var lastRawPlaybackFrame = 0L
    private var playbackWraps = 0L
    private var writtenFrames = 0L

    fun reset(rawPlaybackHead: Int) {
        val raw = rawPlaybackHead.toLong() and UINT32_MASK
        basePlaybackFrame = raw
        lastRawPlaybackFrame = raw
        playbackWraps = 0
        writtenFrames = 0
    }

    fun recordWritten(frames: Int) {
        writtenFrames += frames
    }

    fun queuedFrames(rawPlaybackHead: Int): Long {
        val raw = rawPlaybackHead.toLong() and UINT32_MASK
        if (raw < lastRawPlaybackFrame && lastRawPlaybackFrame - raw > UINT32_HALF_RANGE) {
            playbackWraps += 1
        }
        lastRawPlaybackFrame = raw
        val played = (playbackWraps shl 32) + raw - basePlaybackFrame
        return (writtenFrames - played).coerceAtLeast(0)
    }

    private companion object {
        const val UINT32_MASK = 0xFFFF_FFFFL
        const val UINT32_HALF_RANGE = 0x8000_0000L
    }
}
