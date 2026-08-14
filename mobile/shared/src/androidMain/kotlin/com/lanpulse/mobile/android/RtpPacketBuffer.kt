package com.lanpulse.mobile.android

internal class RtpPacketBuffer(capacity: Int) {
    val bytes = ByteArray(capacity)
    var sequence: Int = 0
    var timestamp: Long = 0
    var ssrc: Long = 0
    var payloadOffset: Int = 0
    var payloadLength: Int = 0
    var arrivalNanos: Long = 0

    fun parseInPlace(length: Int, expectedPayloadType: Int, expectedSsrc: Long? = null): Boolean {
        payloadLength = 0
        if (length < RTP_HEADER_BYTES || length > bytes.size) return false
        val first = bytes[0].toInt() and 0xFF
        if (first ushr 6 != RTP_VERSION) return false
        val csrcCount = first and 0x0F
        val hasPadding = first and 0x20 != 0
        val hasExtension = first and 0x10 != 0
        val payloadType = bytes[1].toInt() and 0x7F
        if (payloadType != expectedPayloadType) return false
        timestamp = readUint32(bytes, 4)
        ssrc = readUint32(bytes, 8)
        if (expectedSsrc != null && ssrc != expectedSsrc) return false

        var offset = RTP_HEADER_BYTES + csrcCount * 4
        if (offset > length) return false
        if (hasExtension) {
            if (offset + 4 > length) return false
            val extensionWords = ((bytes[offset + 2].toInt() and 0xFF) shl 8) or
                (bytes[offset + 3].toInt() and 0xFF)
            offset += 4 + extensionWords * 4
            if (offset > length) return false
        }

        var payloadEnd = length
        if (hasPadding) {
            val paddingBytes = bytes[length - 1].toInt() and 0xFF
            if (paddingBytes == 0 || paddingBytes > payloadEnd - offset) return false
            payloadEnd -= paddingBytes
        }
        if (payloadEnd <= offset) return false
        sequence = ((bytes[2].toInt() and 0xFF) shl 8) or (bytes[3].toInt() and 0xFF)
        payloadOffset = offset
        payloadLength = payloadEnd - offset
        return true
    }

    fun payloadCopy(): ByteArray = bytes.copyOfRange(payloadOffset, payloadOffset + payloadLength)

    private fun readUint32(bytes: ByteArray, offset: Int): Long =
        ((bytes[offset].toLong() and 0xFF) shl 24) or
            ((bytes[offset + 1].toLong() and 0xFF) shl 16) or
            ((bytes[offset + 2].toLong() and 0xFF) shl 8) or
            (bytes[offset + 3].toLong() and 0xFF)

    private companion object {
        const val RTP_VERSION = 2
        const val RTP_HEADER_BYTES = 12
    }
}
