package com.lanpulse.mobile.android

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.os.Process
import com.lanpulse.mobile.AudioConfig
import com.lanpulse.mobile.PlaybackMode
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.SocketException
import java.net.SocketAddress
import java.net.SocketTimeoutException
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.isActive
import kotlinx.coroutines.joinAll
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull

internal class RtpAudioReceiver(
    private val socket: DatagramSocket,
    private val audio: AudioConfig,
    private val playbackMode: PlaybackMode = PlaybackMode.Adaptive,
    private val onStats: (ReceiverStats) -> Unit,
    private val onDiagnostic: (String) -> Unit = {},
    private val enableNack: Boolean = false,
) {
    suspend fun play() {
        validateAudioConfig(audio)
        if (playbackMode == PlaybackMode.Immediate) {
            playDirect()
        } else {
            playBuffered()
        }
    }

    private suspend fun playBuffered() = coroutineScope {
        val bufferPolicy = playbackBufferPolicy(playbackMode)
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
        val maxReceiveGapNanos = AtomicLong(0)
        val rtpSourceAddress = AtomicReference<SocketAddress?>(null)
        repeat(poolSize) {
            check(freePackets.offer(RtpPacketBuffer(packetBufferBytes)))
        }
        val packetSignal = Channel<Unit>(Channel.CONFLATED)

        runCatching {
            socket.receiveBufferSize = maxOf(socket.receiveBufferSize, UDP_RECEIVE_BUFFER_BYTES)
        }
        onDiagnostic(
            "receiver_start local_port=${socket.localPort} receive_buffer=${socket.receiveBufferSize} " +
                "packet_bytes=$expectedPayloadBytes pool_size=$poolSize jitter_slots=$jitterSlots " +
                "playback_mode=${playbackMode.storageValue}",
        )
        val receiveDispatcher = Executors.newSingleThreadExecutor { command ->
            Thread(
                {
                    Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
                    command.run()
                },
                "LanPulseRtpReceive",
            )
        }.asCoroutineDispatcher()
        val receiveJob = launch(receiveDispatcher) {
            val discardBytes = ByteArray(packetBufferBytes)
            val datagram = DatagramPacket(discardBytes, discardBytes.size)
            var reusable: RtpPacketBuffer? = null
            var previousArrivalNanos = 0L
            onDiagnostic(
                "receiver_thread_started name=${Thread.currentThread().name} " +
                    "priority=${Process.getThreadPriority(Process.myTid())}",
            )
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
                    if (previousArrivalNanos != 0L) {
                        maxReceiveGapNanos.updateMax(packet.arrivalNanos - previousArrivalNanos)
                    }
                    previousArrivalNanos = packet.arrivalNanos
                    val valid = packet.parseInPlace(
                        length = datagram.length,
                        expectedPayloadType = audio.payloadType,
                        expectedSsrc = audio.ssrc,
                    ) && packet.payloadLength == expectedPayloadBytes
                    if (valid) {
                        lastValidPacketNanos.set(packet.arrivalNanos)
                        rtpSourceAddress.set(datagram.socketAddress)
                    }
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

        val adaptiveBuffer = AdaptiveJitterController(
            sampleRate = audio.sampleRate,
            packetMs = audio.packetMs,
            initialBufferMs = bufferPolicy.initialBufferMs,
            minBufferMs = bufferPolicy.minBufferMs,
            maxBufferMs = bufferPolicy.maxBufferMs,
        )
        val jitterBuffer = RtpJitterBuffer(
            targetPacketCount = adaptiveBuffer.targetPacketCount,
            maxBufferedPackets = maxBufferedPackets,
            recycle = { packet -> check(freePackets.offer(packet)) },
        )
        val track = createAudioTrack(audio, bufferPolicy)
        onDiagnostic(
            "audio_track_created state=${track.state} session_id=${track.audioSessionId} " +
                "buffer_bytes=${track.bufferSizeInFrames * audio.channels * PCM_16_BYTES_PER_SAMPLE}",
        )
        val silence = ByteArray(expectedPayloadBytes)
        val framesPerPacket = audio.sampleRate * audio.packetMs / 1_000
        val outputBufferPackets = packetsForDuration(bufferPolicy.outputBufferMs, audio.packetMs)
        val backlogMarginPackets =
            packetsForDuration(bufferPolicy.backlogMarginMs, audio.packetMs)
        val bytesPerFrame = audio.channels * PCM_16_BYTES_PER_SAMPLE
        val playbackClock = AudioPlaybackClock()
        val driftController = ClockDriftController(framesPerPacket)
        val frameAdjuster = Pcm16FrameAdjuster(expectedPayloadBytes, bytesPerFrame)
        val nackRequester = if (enableNack) {
            RtpNackRequester(socket, rtpSourceAddress, audio.ssrc)
        } else {
            null
        }
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
        var maxDispatchDelayNanos = 0L
        var maxAudioWriteNanos = 0L
        var outputDroppedBytes = 0L
        var nackRequests = 0L
        var nackRecoveries = 0L
        var requestedNackSequence: Int? = null
        var nackAttempts = 0
        var lastNackNanos = 0L

        fun recycle(packet: RtpPacketBuffer) {
            check(freePackets.offer(packet))
        }

        fun drainAvailablePackets() {
            while (true) {
                val packet = readyPackets.poll() ?: return
                maxDispatchDelayNanos = maxOf(
                    maxDispatchDelayNanos,
                    (System.nanoTime() - packet.arrivalNanos).coerceAtLeast(0L),
                )
                adaptiveBuffer.observe(packet.timestamp, packet.arrivalNanos)
                jitterBuffer.updateTarget(adaptiveBuffer.targetPacketCount)
                jitterBuffer.offer(packet)
                if (jitterBuffer.needsSequenceReset()) return
            }
        }

        fun writeFully(bytes: ByteArray, offset: Int, length: Int) {
            val startedNanos = System.nanoTime()
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
            maxAudioWriteNanos = maxOf(maxAudioWriteNanos, System.nanoTime() - startedNanos)
        }

        fun writePcm(bytes: ByteArray, offset: Int, length: Int, correctDrift: Boolean) {
            if (bufferPolicy.directWrite) {
                val startedNanos = System.nanoTime()
                val written = track.write(
                    bytes,
                    offset,
                    length,
                    AudioTrack.WRITE_NON_BLOCKING,
                )
                if (written < 0) error("AudioTrack write failed: $written")
                maxAudioWriteNanos = maxOf(maxAudioWriteNanos, System.nanoTime() - startedNanos)
                outputDroppedBytes += length - written
                playbackClock.recordWritten(written / bytesPerFrame)
                return
            }

            val correction = if (correctDrift) {
                val queuedFrames = playbackClock.queuedFrames(track.playbackHeadPosition) +
                    jitterBuffer.bufferedSpanPackets().toLong() * framesPerPacket
                val targetFrames = playbackTargetFrames(
                    targetPacketCount = adaptiveBuffer.targetPacketCount,
                    framesPerPacket = framesPerPacket,
                    outputBufferFrames = track.bufferSizeInFrames,
                )
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

        fun concealExpectedPacket() {
            if (jitterBuffer.expectedSequenceNumber() == requestedNackSequence) {
                requestedNackSequence = null
                nackAttempts = 0
            }
            jitterBuffer.concealExpected()
            writePcm(silence, 0, silence.size, correctDrift = true)
            lost += 1
            packetsSinceStats += 1
        }

        try {
            fun requestMissingPacket() {
                val requester = nackRequester ?: return
                val sequence = jitterBuffer.expectedSequenceNumber() ?: return
                val nowNanos = System.nanoTime()
                if (requestedNackSequence != sequence) {
                    requestedNackSequence = sequence
                    nackAttempts = 0
                    lastNackNanos = 0L
                }
                if (
                    nackAttempts >= MAX_NACK_ATTEMPTS ||
                    nowNanos - lastNackNanos < NACK_RETRY_INTERVAL_NANOS
                ) {
                    return
                }
                if (requester.request(sequence)) {
                    nackAttempts += 1
                    nackRequests += 1
                    lastNackNanos = nowNanos
                }
            }

            fun publishStats() {
                val outputQueuedFrames = playbackClock.queuedFrames(track.playbackHeadPosition)
                val softwareQueuedFrames =
                    jitterBuffer.bufferedSpanPackets().toLong() * framesPerPacket
                val queuedFrames = outputQueuedFrames + softwareQueuedFrames
                onStats(
                    ReceiverStats(
                        packetsReceived = received,
                        packetsLost = lost,
                        bufferMs = adaptiveBuffer.targetPacketCount * audio.packetMs,
                        queuedMs = (queuedFrames * 1_000 / audio.sampleRate).toInt(),
                        softwareQueuedMs =
                            (softwareQueuedFrames * 1_000 / audio.sampleRate).toInt(),
                        outputQueuedMs =
                            (outputQueuedFrames * 1_000 / audio.sampleRate).toInt(),
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
                        maxReceiveGapMs = maxReceiveGapNanos.get().toWholeMilliseconds(),
                        maxDispatchDelayMs = maxDispatchDelayNanos.toWholeMilliseconds(),
                        maxAudioWriteMs = maxAudioWriteNanos.toWholeMilliseconds(),
                        outputDroppedBytes = outputDroppedBytes,
                        nackRequests = nackRequests,
                        nackRecoveries = nackRecoveries,
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

                val outputQueuedFrames = playbackClock.queuedFrames(track.playbackHeadPosition)
                val outputQueuedPackets =
                    ((outputQueuedFrames + framesPerPacket - 1) / framesPerPacket).toInt()
                val prebufferPackets =
                    (outputBufferPackets - outputQueuedPackets).coerceIn(
                        0,
                        jitterBuffer.targetPacketCount,
                    )
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
                    val queuedFrames = playbackClock.queuedFrames(track.playbackHeadPosition) +
                        jitterBuffer.bufferedSpanPackets().toLong() * framesPerPacket
                    val targetFrames = adaptiveBuffer.targetPacketCount.toLong() * framesPerPacket
                    recoverFromUnderrun = !bufferPolicy.directWrite &&
                        queuedFrames < targetFrames / 2
                }
                val sequenceRestarted = jitterBuffer.needsSequenceReset()
                val latencyReset = !bufferPolicy.directWrite &&
                    jitterBuffer.exceedsLeadLimit(
                        jitterBuffer.targetPacketCount + backlogMarginPackets,
                    )
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
                    onDiagnostic(
                        "latency_backlog_recovery previous_lead_packets=$previousLead " +
                            "target_packets=${jitterBuffer.targetPacketCount}",
                    )
                    rebuffer("latency_recovery", flushTrack = false)
                    continue
                }
                val packet = jitterBuffer.pollExpected()
                if (packet != null) {
                    if (requestedNackSequence == packet.sequence) {
                        nackRecoveries += 1
                        requestedNackSequence = null
                        nackAttempts = 0
                    }
                    writePcm(
                        packet.bytes,
                        packet.payloadOffset,
                        packet.payloadLength,
                        correctDrift = true,
                    )
                    recycle(packet)
                    received += 1
                    packetsSinceStats += 1
                } else {
                    val hasLaterPacket = jitterBuffer.hasPacketAfterExpected()
                    val queuedFrames = playbackClock.queuedFrames(track.playbackHeadPosition)
                    if (
                        hasLaterPacket &&
                        canWaitForMissingPacket(queuedFrames, audio.sampleRate)
                    ) {
                        requestMissingPacket()
                    }
                    if (
                        hasLaterPacket &&
                        jitterBuffer.shouldConcealExpected() &&
                        !canWaitForMissingPacket(queuedFrames, audio.sampleRate)
                    ) {
                        concealExpectedPacket()
                    } else {
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
                                    "stream_timeout " +
                                        "idle_ms=${streamIdleNanos / NANOS_PER_MILLISECOND} " +
                                        "received=$received lost=$lost",
                                )
                                throw SocketTimeoutException("RTP audio stream timed out")
                            }
                        } else {
                            drainAvailablePackets()
                        }
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
                receiveDispatcher.close()
            }
        }
    }

    private suspend fun playDirect() {
        val bufferPolicy = playbackBufferPolicy(PlaybackMode.Immediate)
        val expectedPayloadBytes = expectedPayloadBytes(audio)
        val packet = RtpPacketBuffer(expectedPayloadBytes + MAX_RTP_HEADER_BYTES)
        val datagram = DatagramPacket(packet.bytes, packet.bytes.size)
        val track = createAudioTrack(audio, bufferPolicy)
        val playbackClock = AudioPlaybackClock()
        val metrics = DirectPlaybackMetrics()
        val coroutineContext = currentCoroutineContext()
        val statsIntervalNanos = STATS_UPDATE_INTERVAL_MS * NANOS_PER_MILLISECOND
        var lastValidPacketNanos = System.nanoTime()
        var lastStatsNanos = lastValidPacketNanos
        var started = false

        runCatching {
            socket.receiveBufferSize = maxOf(socket.receiveBufferSize, UDP_RECEIVE_BUFFER_BYTES)
        }
        socket.soTimeout = DIRECT_RECEIVE_POLL_MS
        onDiagnostic(
            "receiver_start local_port=${socket.localPort} receive_buffer=${socket.receiveBufferSize} " +
                "packet_bytes=$expectedPayloadBytes playback_mode=${playbackMode.storageValue} " +
                "path=direct",
        )
        onDiagnostic(
            "receiver_thread_started name=${Thread.currentThread().name} " +
                "priority=${Process.getThreadPriority(Process.myTid())}",
        )
        onDiagnostic(
            "audio_track_created state=${track.state} session_id=${track.audioSessionId} " +
                "buffer_bytes=${track.bufferSizeInFrames * audio.channels * PCM_16_BYTES_PER_SAMPLE}",
        )

        try {
            while (coroutineContext.isActive) {
                datagram.setData(packet.bytes, 0, packet.bytes.size)
                try {
                    socket.receive(datagram)
                } catch (_: SocketTimeoutException) {
                    val idleNanos = System.nanoTime() - lastValidPacketNanos
                    if (idleNanos >= STREAM_TIMEOUT_MS * NANOS_PER_MILLISECOND) {
                        onDiagnostic(
                            "stream_timeout idle_ms=${idleNanos / NANOS_PER_MILLISECOND} " +
                                "received=${metrics.packetsReceived}",
                        )
                        throw SocketTimeoutException("RTP audio stream timed out")
                    }
                    continue
                }

                val arrivalNanos = System.nanoTime()
                metrics.observeArrival(arrivalNanos)
                packet.arrivalNanos = arrivalNanos
                val valid = packet.parseInPlace(
                    length = datagram.length,
                    expectedPayloadType = audio.payloadType,
                    expectedSsrc = audio.ssrc,
                ) && packet.payloadLength == expectedPayloadBytes
                if (!valid) {
                    metrics.invalidPackets += 1
                    continue
                }

                lastValidPacketNanos = arrivalNanos
                val writeStartedNanos = System.nanoTime()
                val written = track.write(
                    packet.bytes,
                    packet.payloadOffset,
                    packet.payloadLength,
                    AudioTrack.WRITE_NON_BLOCKING,
                )
                if (written < 0) error("AudioTrack write failed: $written")
                metrics.maxAudioWriteNanos = maxOf(
                    metrics.maxAudioWriteNanos,
                    System.nanoTime() - writeStartedNanos,
                )
                metrics.outputDroppedBytes += packet.payloadLength - written
                metrics.packetsReceived += 1
                playbackClock.recordWritten(
                    written / (audio.channels * PCM_16_BYTES_PER_SAMPLE),
                )
                if (!started && written > 0) {
                    track.play()
                    started = true
                    onStats(metrics.snapshot(audio, track, playbackClock))
                }

                if (started && arrivalNanos - lastStatsNanos >= statsIntervalNanos) {
                    lastStatsNanos = arrivalNanos
                    onStats(metrics.snapshot(audio, track, playbackClock))
                }
            }
        } catch (error: SocketException) {
            if (coroutineContext.isActive && !socket.isClosed) throw error
        } finally {
            onDiagnostic(
                "receiver_stop received=${metrics.packetsReceived} " +
                    "invalid=${metrics.invalidPackets} " +
                    "underruns=${track.underrunCount} path=direct",
            )
            socket.close()
            runCatching { track.pause() }
            runCatching { track.flush() }
            runCatching { track.release() }
        }
    }

    private fun createAudioTrack(
        audio: AudioConfig,
        bufferPolicy: PlaybackBufferPolicy,
    ): AudioTrack {
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
        val outputBufferPackets = packetsForDuration(bufferPolicy.outputBufferMs, audio.packetMs)
        val bufferBytes = maxOf(minBuffer, expectedPayloadBytes(audio) * outputBufferPackets)
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
        private const val DIRECT_RECEIVE_POLL_MS = 100
        private const val MAX_NACK_ATTEMPTS = 3
        private const val NACK_RETRY_INTERVAL_NANOS = 20_000_000L

        fun audioAttributes(): AudioAttributes = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_MEDIA)
            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
            .build()
    }
}

private class DirectPlaybackMetrics {
    var packetsReceived = 0L
    var invalidPackets = 0L
    var outputDroppedBytes = 0L
    var maxReceiveGapNanos = 0L
    var maxAudioWriteNanos = 0L
    private var previousArrivalNanos = 0L

    fun observeArrival(arrivalNanos: Long) {
        if (previousArrivalNanos != 0L) {
            maxReceiveGapNanos = maxOf(maxReceiveGapNanos, arrivalNanos - previousArrivalNanos)
        }
        previousArrivalNanos = arrivalNanos
    }

    fun snapshot(
        audio: AudioConfig,
        track: AudioTrack,
        playbackClock: AudioPlaybackClock,
    ): ReceiverStats {
        val outputQueuedFrames = playbackClock.queuedFrames(track.playbackHeadPosition)
        return ReceiverStats(
            packetsReceived = packetsReceived,
            packetsLost = 0,
            bufferMs = audio.packetMs,
            queuedMs = (outputQueuedFrames * 1_000 / audio.sampleRate).toInt(),
            softwareQueuedMs = 0,
            outputQueuedMs = (outputQueuedFrames * 1_000 / audio.sampleRate).toInt(),
            jitterMs = 0.0,
            audioUnderruns = track.underrunCount,
            driftInsertedFrames = 0,
            driftDroppedFrames = 0,
            invalidPackets = invalidPackets,
            receiveQueueOverflows = 0,
            packetPoolExhausted = 0,
            duplicatePackets = 0,
            latePackets = 0,
            replacedPackets = 0,
            prunedPackets = 0,
            maxReceiveGapMs = maxReceiveGapNanos.toWholeMilliseconds(),
            maxDispatchDelayMs = 0,
            maxAudioWriteMs = maxAudioWriteNanos.toWholeMilliseconds(),
            outputDroppedBytes = outputDroppedBytes,
            nackRequests = 0,
            nackRecoveries = 0,
        )
    }
}

private fun AtomicLong.updateMax(candidate: Long) {
    var observed = get()
    while (candidate > observed && !compareAndSet(observed, candidate)) {
        observed = get()
    }
}

private fun Long.toWholeMilliseconds(): Int =
    (this / 1_000_000L).coerceAtMost(Int.MAX_VALUE.toLong()).toInt()
