package com.lanpulse.mobile.android

internal class Pcm16FrameAdjuster(maxPayloadBytes: Int, private val bytesPerFrame: Int) {
    val output = ByteArray(maxPayloadBytes + bytesPerFrame)

    fun adjust(input: ByteArray, offset: Int, length: Int, correction: Int): Int {
        require(correction == -1 || correction == 1)
        require(length % bytesPerFrame == 0)
        val frames = length / bytesPerFrame
        require(frames >= 2)
        return if (correction < 0) {
            dropFrameSmoothly(input, offset, length, frames)
        } else {
            insertFrameSmoothly(input, offset, length, frames)
        }
    }

    private fun dropFrameSmoothly(input: ByteArray, offset: Int, length: Int, frames: Int): Int {
        val crossfadeFrames = minOf(CROSSFADE_FRAMES, frames - 1)
        val startFrame = (frames - crossfadeFrames - 1) / 2
        val startByte = startFrame * bytesPerFrame
        input.copyInto(output, 0, offset, offset + startByte)
        repeat(crossfadeFrames) { frameOffset ->
            blendFrame(
                input = input,
                firstFrame = offset + (startFrame + frameOffset) * bytesPerFrame,
                secondFrame = offset + (startFrame + frameOffset + 1) * bytesPerFrame,
                outputFrame = (startFrame + frameOffset) * bytesPerFrame,
                numerator = frameOffset + 1,
                denominator = crossfadeFrames,
            )
        }
        val outputTail = (startFrame + crossfadeFrames) * bytesPerFrame
        val inputTail = offset + (startFrame + crossfadeFrames + 1) * bytesPerFrame
        input.copyInto(output, outputTail, inputTail, offset + length)
        return length - bytesPerFrame
    }

    private fun insertFrameSmoothly(input: ByteArray, offset: Int, length: Int, frames: Int): Int {
        val crossfadeFrames = minOf(CROSSFADE_FRAMES, frames - 1)
        val startFrame = maxOf(1, (frames - crossfadeFrames) / 2)
        val startByte = startFrame * bytesPerFrame
        input.copyInto(output, 0, offset, offset + startByte)
        repeat(crossfadeFrames) { frameOffset ->
            blendFrame(
                input = input,
                firstFrame = offset + (startFrame + frameOffset) * bytesPerFrame,
                secondFrame = offset + (startFrame + frameOffset - 1) * bytesPerFrame,
                outputFrame = (startFrame + frameOffset) * bytesPerFrame,
                numerator = frameOffset + 1,
                denominator = crossfadeFrames,
            )
        }
        val outputTail = (startFrame + crossfadeFrames) * bytesPerFrame
        val inputTail = offset + (startFrame + crossfadeFrames - 1) * bytesPerFrame
        input.copyInto(output, outputTail, inputTail, offset + length)
        return length + bytesPerFrame
    }

    private fun blendFrame(
        input: ByteArray,
        firstFrame: Int,
        secondFrame: Int,
        outputFrame: Int,
        numerator: Int,
        denominator: Int,
    ) {
        for (sampleOffset in 0 until bytesPerFrame step PCM_16_BYTES_PER_SAMPLE) {
            val first = readInt16(input, firstFrame + sampleOffset)
            val second = readInt16(input, secondFrame + sampleOffset)
            val blended = (first * (denominator - numerator) + second * numerator) / denominator
            writeInt16(output, outputFrame + sampleOffset, blended)
        }
    }

    private fun readInt16(bytes: ByteArray, offset: Int): Int =
        (((bytes[offset + 1].toInt() and 0xFF) shl 8) or (bytes[offset].toInt() and 0xFF)).toShort().toInt()

    private fun writeInt16(bytes: ByteArray, offset: Int, value: Int) {
        bytes[offset] = value.toByte()
        bytes[offset + 1] = (value shr 8).toByte()
    }

    private companion object {
        const val CROSSFADE_FRAMES = 32
    }
}
