package com.lanpulse.mobile.android

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import com.lanpulse.mobile.AudioConfig
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.SocketTimeoutException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

internal class RtpAudioReceiver(
    private val socket: DatagramSocket,
    private val audio: AudioConfig,
    private val onStats: (ReceiverStats) -> Unit,
) {
    suspend fun play() = coroutineScope {
        validateAudioConfig(audio)

        val expectedPayloadBytes = expectedPayloadBytes(audio)
        val packetBufferBytes = expectedPayloadBytes + MAX_RTP_HEADER_BYTES
        val maxBufferedPackets = packetsForDuration(MAX_BUFFER_MS, audio.packetMs)
        val jitterSlots = nextPowerOfTwo(maxBufferedPackets * STORED_PACKET_MULTIPLIER)
        val poolSize = PACKET_QUEUE_CAPACITY + jitterSlots + PACKET_POOL_HEADROOM
        val freePackets = SpscRing<RtpPacketBuffer>(nextPowerOfTwo(poolSize))
        val readyPackets = SpscRing<RtpPacketBuffer>(PACKET_QUEUE_CAPACITY)
        repeat(poolSize) {
            check(freePackets.offer(RtpPacketBuffer(packetBufferBytes)))
        }
        val packetSignal = Channel<Unit>(Channel.CONFLATED)

        runCatching {
            socket.receiveBufferSize = maxOf(socket.receiveBufferSize, UDP_RECEIVE_BUFFER_BYTES)
        }
        val receiveJob = launch(Dispatchers.IO) {
            val discardBytes = ByteArray(packetBufferBytes)
            val datagram = DatagramPacket(discardBytes, discardBytes.size)
            var reusable: RtpPacketBuffer? = null
            while (isActive) {
                val packet = reusable ?: freePackets.poll()
                val receiveBytes = packet?.bytes ?: discardBytes
                datagram.setData(receiveBytes, 0, receiveBytes.size)
                socket.receive(datagram)
                if (packet == null) continue

                packet.arrivalNanos = System.nanoTime()
                val valid = packet.parseInPlace(
                    length = datagram.length,
                    expectedPayloadType = audio.payloadType,
                    expectedSsrc = audio.ssrc,
                ) && packet.payloadLength == expectedPayloadBytes
                if (valid && readyPackets.offer(packet)) {
                    reusable = null
                    packetSignal.trySend(Unit)
                } else {
                    reusable = packet
                }
            }
        }

        val adaptiveBuffer = AdaptiveJitterController(audio.sampleRate, audio.packetMs)
        val jitterBuffer = RtpJitterBuffer(
            targetPacketCount = adaptiveBuffer.targetPacketCount,
            maxBufferedPackets = maxBufferedPackets,
            recycle = { packet -> check(freePackets.offer(packet)) },
        )
        val track = createAudioTrack(audio, maxBufferedPackets)
        val silence = ByteArray(expectedPayloadBytes)
        val framesPerPacket = audio.sampleRate * audio.packetMs / 1_000
        val bytesPerFrame = audio.channels * PCM_16_BYTES_PER_SAMPLE
        val playbackClock = AudioPlaybackClock()
        val driftController = ClockDriftController(framesPerPacket)
        val frameAdjuster = Pcm16FrameAdjuster(expectedPayloadBytes, bytesPerFrame)
        val statsPacketInterval = packetsForDuration(STATS_UPDATE_INTERVAL_MS, audio.packetMs)
        val underrunCheckInterval = packetsForDuration(UNDERRUN_CHECK_INTERVAL_MS, audio.packetMs)
        var received = 0L
        var lost = 0L
        var packetsSinceStats = 0
        var observedUnderruns = track.underrunCount
        var packetsSinceUnderrunCheck = 0

        fun recycle(packet: RtpPacketBuffer) {
            check(freePackets.offer(packet))
        }

        fun drainAvailablePackets() {
            while (true) {
                val packet = readyPackets.poll() ?: return
                adaptiveBuffer.observe(packet.timestamp, packet.arrivalNanos)
                jitterBuffer.updateTarget(adaptiveBuffer.targetPacketCount)
                jitterBuffer.offer(packet)
            }
        }

        fun writeFully(bytes: ByteArray, offset: Int, length: Int) {
            var writtenBytes = 0
            while (writtenBytes < length) {
                val written = track.write(
                    bytes,
                    offset + writtenBytes,
                    length - writtenBytes,
                    AudioTrack.WRITE_BLOCKING,
                )
                if (written <= 0) error("AudioTrack write failed: $written")
                writtenBytes += written
            }
        }

        fun writePcm(bytes: ByteArray, offset: Int, length: Int, correctDrift: Boolean) {
            val correction = if (correctDrift) {
                val queuedFrames = playbackClock.queuedFrames(track.playbackHeadPosition)
                val targetFrames = adaptiveBuffer.targetPacketCount.toLong() * framesPerPacket
                driftController.correction(queuedFrames, targetFrames)
            } else {
                0
            }
            if (correction == 0) {
                writeFully(bytes, offset, length)
                playbackClock.recordWritten(length / bytesPerFrame)
            } else {
                val adjustedLength = frameAdjuster.adjust(bytes, offset, length, correction)
                writeFully(frameAdjuster.output, 0, adjustedLength)
                playbackClock.recordWritten(adjustedLength / bytesPerFrame)
            }
        }

        try {
            fun publishStats() {
                onStats(
                    ReceiverStats(
                        packetsReceived = received,
                        packetsLost = lost,
                        bufferMs = adaptiveBuffer.targetPacketCount * audio.packetMs,
                    ),
                )
                packetsSinceStats = 0
            }

            suspend fun rebuffer() {
                if (track.playState == AudioTrack.PLAYSTATE_PLAYING) track.pause()
                track.flush()
                playbackClock.reset(track.playbackHeadPosition)
                drainAvailablePackets()
                val streamReady = withTimeoutOrNull(STREAM_TIMEOUT_MS.toLong()) {
                    while (!jitterBuffer.readyToStart()) {
                        drainAvailablePackets()
                        if (!jitterBuffer.readyToStart()) packetSignal.receive()
                    }
                    true
                } ?: false
                if (!streamReady) throw SocketTimeoutException("RTP audio stream timed out")

                val prebufferPackets = jitterBuffer.targetPacketCount
                jitterBuffer.startNearLive()
                repeat(prebufferPackets) {
                    val packet = jitterBuffer.pollExpected()
                    if (packet == null) {
                        writePcm(silence, 0, silence.size, correctDrift = false)
                        lost += 1
                    } else {
                        writePcm(
                            packet.bytes,
                            packet.payloadOffset,
                            packet.payloadLength,
                            correctDrift = false,
                        )
                        recycle(packet)
                        received += 1
                    }
                }
                track.play()
                observedUnderruns = track.underrunCount
                publishStats()
            }

            rebuffer()

            while (isActive) {
                drainAvailablePackets()
                packetsSinceUnderrunCheck += 1
                val playbackStarved = if (packetsSinceUnderrunCheck >= underrunCheckInterval) {
                    packetsSinceUnderrunCheck = 0
                    val currentUnderruns = track.underrunCount
                    val starved = currentUnderruns > observedUnderruns
                    observedUnderruns = currentUnderruns
                    starved
                } else {
                    false
                }
                if (playbackStarved) {
                    adaptiveBuffer.onUnderrun(System.nanoTime())
                    jitterBuffer.updateTarget(adaptiveBuffer.targetPacketCount)
                }
                val sequenceRestarted = jitterBuffer.needsSequenceReset()
                if (playbackStarved || jitterBuffer.needsLatencyReset() || sequenceRestarted) {
                    if (sequenceRestarted) jitterBuffer.reset()
                    rebuffer()
                    continue
                }

                val packet = jitterBuffer.pollExpected()
                if (packet != null) {
                    writePcm(
                        packet.bytes,
                        packet.payloadOffset,
                        packet.payloadLength,
                        correctDrift = true,
                    )
                    recycle(packet)
                    received += 1
                    packetsSinceStats += 1
                } else if (jitterBuffer.shouldConcealExpected()) {
                    jitterBuffer.concealExpected()
                    writePcm(silence, 0, silence.size, correctDrift = true)
                    lost += 1
                    packetsSinceStats += 1
                } else {
                    val packetArrived = withTimeoutOrNull(missingPacketWaitMs(audio.packetMs)) {
                        packetSignal.receive()
                        true
                    } ?: false
                    if (!packetArrived) {
                        rebuffer()
                    } else {
                        drainAvailablePackets()
                    }
                    continue
                }

                if (packetsSinceStats >= statsPacketInterval) publishStats()
            }
        } finally {
            socket.close()
            receiveJob.cancel()
            jitterBuffer.reset()
            track.pause()
            track.flush()
            track.release()
            packetSignal.close()
        }
    }

    private fun createAudioTrack(audio: AudioConfig, maxBufferedPackets: Int): AudioTrack {
        val channelMask = if (audio.channels == 1) {
            AudioFormat.CHANNEL_OUT_MONO
        } else {
            AudioFormat.CHANNEL_OUT_STEREO
        }
        val format = AudioFormat.Builder()
            .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
            .setSampleRate(audio.sampleRate)
            .setChannelMask(channelMask)
            .build()
        val minBuffer = AudioTrack.getMinBufferSize(
            audio.sampleRate,
            channelMask,
            AudioFormat.ENCODING_PCM_16BIT,
        )
        check(minBuffer > 0) { "AudioTrack does not support this audio format" }
        val bufferBytes = maxOf(minBuffer, expectedPayloadBytes(audio) * maxBufferedPackets)
        return AudioTrack.Builder()
            .setAudioAttributes(audioAttributes())
            .setAudioFormat(format)
            .setTransferMode(AudioTrack.MODE_STREAM)
            .setBufferSizeInBytes(bufferBytes)
            .setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
            .build()
            .also { check(it.state == AudioTrack.STATE_INITIALIZED) { "AudioTrack initialization failed" } }
    }

    companion object {
        fun audioAttributes(): AudioAttributes = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_MEDIA)
            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
            .build()
    }
}
