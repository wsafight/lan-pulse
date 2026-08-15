package com.lanpulse.mobile.android

import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.SocketAddress
import java.util.concurrent.atomic.AtomicReference

internal class RtpNackRequester(
    private val socket: DatagramSocket,
    private val sourceAddress: AtomicReference<SocketAddress?>,
    private val ssrc: Long,
) {
    private val bytes = ByteArray(RTP_NACK_PACKET_BYTES)
    private val datagram = DatagramPacket(bytes, bytes.size)

    fun request(sequence: Int): Boolean {
        val target = sourceAddress.get() ?: return false
        encodeRtpNack(bytes, sequence, ssrc)
        datagram.socketAddress = target
        return runCatching { socket.send(datagram) }.isSuccess
    }
}

internal fun encodeRtpNack(target: ByteArray, sequence: Int, ssrc: Long) {
    require(target.size >= RTP_NACK_PACKET_BYTES)
    target[0] = 'L'.code.toByte()
    target[1] = 'P'.code.toByte()
    target[2] = 'N'.code.toByte()
    target[3] = 'K'.code.toByte()
    target[4] = RTP_NACK_VERSION
    target[5] = 0
    target[6] = (sequence ushr 8).toByte()
    target[7] = sequence.toByte()
    target[8] = (ssrc ushr 24).toByte()
    target[9] = (ssrc ushr 16).toByte()
    target[10] = (ssrc ushr 8).toByte()
    target[11] = ssrc.toByte()
}

internal const val RTP_NACK_PACKET_BYTES = 12
private const val RTP_NACK_VERSION: Byte = 1
