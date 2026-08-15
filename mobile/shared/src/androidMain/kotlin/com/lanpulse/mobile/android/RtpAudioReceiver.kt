package com.lanpulse.mobile.android

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import com.lanpulse.mobile.AudioConfig
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.SocketException
import java.net.SocketTimeoutException
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.isActive
import kotlinx.coroutines.joinAll
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull

internal class RtpAudioReceiver(
    private val socket: DatagramSocket,
    private val audio: AudioConfig,
    private val onStats: (ReceiverStats) -> Unit,
    private val onDiagnostic: (String) -> Unit = {},
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
        val invalidPackets = AtomicLong(0)
        val receiveQueueOverflows = AtomicLong(0)
        val packetPoolExhausted = AtomicLong(0)
        val lastValidPacketNanos = AtomicLong(System.nanoTime())
        repeat(poolSize) {
            check(freePackets.offer(RtpPacketBuffer(packetBufferBytes)))
        }
        val packetSignal = Channel<Unit>(Channel.CONFLATED)

        runCatching {
            socket.receiveBufferSize = maxOf(socket.receiveBufferSize, UDP_RECEIVE_BUFFER_BYTES)
        }
        onDiagnostic(
            "receiver_start local_port=${socket.localPort} receive_buffer=${socket.receiveBufferSize} " +
                "packet_bytes=$expectedPayloadBytes pool_size=$poolSize jitter_slots=$jitterSlots",
        )
        val receiveJob = launch(Dispatchers.IO) {
            val discardBytes = ByteArray(packetBufferBytes)
            val datagram = DatagramPacket(discardBytes, discardBytes.size)
            var reusable: RtpPacketBuffer? = null
            try {
                while (isActive) {
                    val packet = reusable ?: freePackets.poll()
                    val receiveBytes = packet?.bytes ?: discardBytes
                    datagram.setData(receiveBytes, 0, receiveBytes.size)
                    socket.receive(datagram)
                    if (packet == null) {
                        packetPoolExhausted.incrementAndGet()
                        continue
                    }

                    packet.arrivalNanos = System.nanoTime()
                    val valid = packet.parseInPlace(
                        length = datagram.length,
                        expectedPayloadType = audio.payloadType,
                        expectedSsrc = audio.ssrc,
                    ) && packet.payloadLength == expectedPayloadBytes
                    if (valid) lastValidPacketNanos.set(packet.arrivalNanos)
                    if (valid && readyPackets.offer(packet)) {
                        reusable = null
                        packetSignal.trySend(Unit)
                    } else {
                        if (valid) {
                            receiveQueueOverflows.incrementAndGet()
                        } else {
                            invalidPackets.incrementAndGet()
                        }
                        reusable = packet
                    }
                }
            } catch (error: SocketException) {
                if (isActive && !socket.isClosed) {
                    onDiagnostic("receiver_io_error type=SocketException message=${error.message}")
                    throw error
                }
            } catch (error: Exception) {
                onDiagnostic(
                    "receiver_io_error type=${error::class.java.simpleName} message=${error.message}",
                )
                throw error
            }
        }

        val adaptiveBuffer = AdaptiveJitterController(audio.sampleRate, audio.packetMs)
        val jitterBuffer = RtpJitterBuffer(
            targetPacketCount = adaptiveBuffer.targetPacketCount,
            maxBufferedPackets = maxBufferedPackets,
            recycle = { packet -> check(freePackets.offer(packet)) },
        )
        val track = createAudioTrack(audio, maxBufferedPackets)
        onDiagnostic(
            "audio_track_created state=${track.state} session_id=${track.audioSessionId} " +
                "buffer_bytes=${track.bufferSizeInFrames * audio.channels * PCM_16_BYTES_PER_SAMPLE}",
        )
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
        var underrunDelta = 0
        var driftInsertedFrames = 0L
        var driftDroppedFrames = 0L

        fun recycle(packet: RtpPacketBuffer) {
            check(freePackets.offer(packet))
        }

        fun drainAvailablePackets() {
            while (true) {
                val packet = readyPackets.poll() ?: return
                adaptiveBuffer.observe(packet.timestamp, packet.arrivalNanos)
                jitterBuffer.updateTarget(adaptiveBuffer.targetPacketCount)
                jitterBuffer.offer(packet)
                if (jitterBuffer.needsSequenceReset()) return
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
                if (correction > 0) {
                    driftInsertedFrames += 1
                } else {
                    driftDroppedFrames += 1
                }
                val adjustedLength = frameAdjuster.adjust(bytes, offset, length, correction)
                writeFully(frameAdjuster.output, 0, adjustedLength)
                playbackClock.recordWritten(adjustedLength / bytesPerFrame)
            }
        }

        try {
            fun publishStats() {
                val queuedFrames = playbackClock.queuedFrames(track.playbackHeadPosition)
                onStats(
                    ReceiverStats(
                        packetsReceived = received,
                        packetsLost = lost,
                        bufferMs = adaptiveBuffer.targetPacketCount * audio.packetMs,
                        queuedMs = (queuedFrames * 1_000 / audio.sampleRate).toInt(),
                        jitterMs = adaptiveBuffer.jitterMs,
                        audioUnderruns = track.underrunCount,
                        driftInsertedFrames = driftInsertedFrames,
                        driftDroppedFrames = driftDroppedFrames,
                        invalidPackets = invalidPackets.get(),
                        receiveQueueOverflows = receiveQueueOverflows.get(),
                        packetPoolExhausted = packetPoolExhausted.get(),
                        duplicatePackets = jitterBuffer.duplicatePackets,
                        latePackets = jitterBuffer.latePackets,
                        replacedPackets = jitterBuffer.replacedPackets,
                        prunedPackets = jitterBuffer.prunedPackets,
                    ),
                )
                packetsSinceStats = 0
            }

            suspend fun rebuffer(reason: String, flushTrack: Boolean) {
                onDiagnostic(
                    "rebuffer_start reason=$reason mode=${if (flushTrack) "flush" else "soft"} " +
                        "target_packets=${jitterBuffer.targetPacketCount} " +
                        "received=$received lost=$lost underruns=${track.underrunCount}",
                )
                if (flushTrack) {
                    if (track.playState == AudioTrack.PLAYSTATE_PLAYING) track.pause()
                    track.flush()
                    playbackClock.reset(track.playbackHeadPosition)
                }
                driftController.reset()
                drainAvailablePackets()
                val streamReady = withTimeoutOrNull(STREAM_TIMEOUT_MS.toLong()) {
                    while (!jitterBuffer.readyToStart()) {
                        drainAvailablePackets()
                        if (!jitterBuffer.readyToStart()) packetSignal.receive()
                    }
                    true
                } ?: false
                if (!streamReady) {
                    onDiagnostic("rebuffer_timeout reason=$reason")
                    throw SocketTimeoutException("RTP audio stream timed out")
                }

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
                onDiagnostic(
                    "rebuffer_complete reason=$reason prebuffer_packets=$prebufferPackets " +
                        "underruns=$observedUnderruns",
                )
            }

            rebuffer("initial", flushTrack = true)

            while (isActive) {
                drainAvailablePackets()
                packetsSinceUnderrunCheck += 1
                val playbackStarved = if (packetsSinceUnderrunCheck >= underrunCheckInterval) {
                    packetsSinceUnderrunCheck = 0
                    val currentUnderruns = track.underrunCount
                    val starved = currentUnderruns > observedUnderruns
                    underrunDelta = if (starved) currentUnderruns - observedUnderruns else 0
                    observedUnderruns = currentUnderruns
                    starved
                } else {
                    false
                }
                var recoverFromUnderrun = false
                if (playbackStarved) {
                    onDiagnostic(
                        "audio_underrun count=${track.underrunCount} " +
                            "target_packets=${adaptiveBuffer.targetPacketCount}",
                    )
                    adaptiveBuffer.onUnderrun(
                        nowNanos = System.nanoTime(),
                        severityPackets = underrunDelta,
                    )
                    jitterBuffer.updateTarget(adaptiveBuffer.targetPacketCount)
                    val queuedFrames = playbackClock.queuedFrames(track.playbackHeadPosition)
                    val targetFrames = adaptiveBuffer.targetPacketCount.toLong() * framesPerPacket
                    recoverFromUnderrun = queuedFrames < targetFrames / 2
                }
                val sequenceRestarted = jitterBuffer.needsSequenceReset()
                val latencyReset = jitterBuffer.needsLatencyReset()
                if (sequenceRestarted) {
                    onDiagnostic(
                        "sequence_discontinuity_detected " +
                            "target_packets=${jitterBuffer.targetPacketCount}",
                    )
                    jitterBuffer.reset()
                    adaptiveBuffer.resetStreamTiming()
                    rebuffer("sequence_resync", flushTrack = false)
                    continue
                }
                if (recoverFromUnderrun) {
                    rebuffer("underrun_recovery", flushTrack = false)
                    continue
                }
                if (latencyReset && jitterBuffer.readyToStart()) {
                    val previousLead = jitterBuffer.bufferedLeadPackets()
                    adaptiveBuffer.onBacklog(System.nanoTime())
                    jitterBuffer.updateTarget(adaptiveBuffer.targetPacketCount)
                    onDiagnostic(
                        "latency_backlog_recovery previous_lead_packets=$previousLead " +
                            "target_packets=${jitterBuffer.targetPacketCount}",
                    )
                    rebuffer("latency_recovery", flushTrack = false)
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
                    val queuedFrames = playbackClock.queuedFrames(track.playbackHeadPosition)
                    val waitMs = missingPacketWaitMs(
                        packetMs = audio.packetMs,
                        queuedFrames = queuedFrames,
                        sampleRate = audio.sampleRate,
                    )
                    val packetArrived = withTimeoutOrNull(waitMs) {
                        packetSignal.receive()
                        true
                    } ?: false
                    if (!packetArrived) {
                        val streamIdleNanos = System.nanoTime() - lastValidPacketNanos.get()
                        if (streamIdleNanos >= STREAM_TIMEOUT_MS * NANOS_PER_MILLISECOND) {
                            onDiagnostic(
                                "stream_timeout idle_ms=${streamIdleNanos / NANOS_PER_MILLISECOND} " +
                                    "received=$received lost=$lost",
                            )
                            throw SocketTimeoutException("RTP audio stream timed out")
                        }
                        val queuedAfterWait =
                            playbackClock.queuedFrames(track.playbackHeadPosition)
                        val concealPackets = concealmentPacketCount(
                            packetMs = audio.packetMs,
                            queuedFrames = queuedAfterWait,
                            sampleRate = audio.sampleRate,
                        )
                        repeat(concealPackets) {
                            jitterBuffer.concealExpected()
                            writePcm(silence, 0, silence.size, correctDrift = true)
                            lost += 1
                            packetsSinceStats += 1
                        }
                    } else {
                        drainAvailablePackets()
                    }
                    continue
                }

                if (packetsSinceStats >= statsPacketInterval) publishStats()
            }
        } finally {
            onDiagnostic(
                "receiver_stop received=$received lost=$lost invalid=${invalidPackets.get()} " +
                    "queue_overflows=${receiveQueueOverflows.get()} " +
                    "pool_exhausted=${packetPoolExhausted.get()} underruns=${track.underrunCount}",
            )
            receiveJob.cancel()
            socket.close()
            withContext(NonCancellable) {
                joinAll(receiveJob)
                jitterBuffer.reset()
                runCatching { track.pause() }
                runCatching { track.flush() }
                runCatching { track.release() }
                packetSignal.close()
            }
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
        private const val NANOS_PER_MILLISECOND = 1_000_000L

        fun audioAttributes(): AudioAttributes = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_MEDIA)
            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
            .build()
    }
}
