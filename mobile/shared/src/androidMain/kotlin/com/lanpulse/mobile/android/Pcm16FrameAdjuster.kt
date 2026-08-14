package com.lanpulse.mobile.android

internal class Pcm16FrameAdjuster(maxPayloadBytes: Int, private val bytesPerFrame: Int) {
    val output = ByteArray(maxPayloadBytes + bytesPerFrame)

    fun adjust(input: ByteArray, offset: Int, length: Int, correction: Int): Int {
        require(correction == -1 || correction == 1)
        require(length % bytesPerFrame == 0)
        val frames = length / bytesPerFrame
        require(frames >= 2)
        val splitFrame = frames / 2
        val splitByte = splitFrame * bytesPerFrame
        input.copyInto(output, 0, offset, offset + splitByte)
        return if (correction < 0) {
            input.copyInto(
                output,
                splitByte,
                offset + splitByte + bytesPerFrame,
                offset + length,
            )
            length - bytesPerFrame
        } else {
            val previousFrame = offset + splitByte - bytesPerFrame
            val nextFrame = offset + splitByte
            for (sampleOffset in 0 until bytesPerFrame step PCM_16_BYTES_PER_SAMPLE) {
                val first = readInt16(input, previousFrame + sampleOffset)
                val second = readInt16(input, nextFrame + sampleOffset)
                writeInt16(output, splitByte + sampleOffset, (first + second) / 2)
            }
            input.copyInto(
                output,
                splitByte + bytesPerFrame,
                offset + splitByte,
                offset + length,
            )
            length + bytesPerFrame
        }
    }

    private fun readInt16(bytes: ByteArray, offset: Int): Int =
        (((bytes[offset + 1].toInt() and 0xFF) shl 8) or (bytes[offset].toInt() and 0xFF)).toShort().toInt()

    private fun writeInt16(bytes: ByteArray, offset: Int, value: Int) {
        bytes[offset] = value.toByte()
        bytes[offset + 1] = (value shr 8).toByte()
    }
}
