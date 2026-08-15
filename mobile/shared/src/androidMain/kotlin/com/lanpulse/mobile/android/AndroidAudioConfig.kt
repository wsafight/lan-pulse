package com.lanpulse.mobile.android

import com.lanpulse.mobile.AudioConfig
import com.lanpulse.mobile.PlaybackMode

internal data class ReceiverStats(
    val packetsReceived: Long,
    val packetsLost: Long,
    val bufferMs: Int,
    val queuedMs: Int,
    val softwareQueuedMs: Int,
    val outputQueuedMs: Int,
    val jitterMs: Double,
    val audioUnderruns: Int,
    val driftInsertedFrames: Long,
    val driftDroppedFrames: Long,
    val invalidPackets: Long,
    val receiveQueueOverflows: Long,
    val packetPoolExhausted: Long,
    val duplicatePackets: Long,
    val latePackets: Long,
    val replacedPackets: Long,
    val prunedPackets: Long,
    val maxReceiveGapMs: Int,
    val maxDispatchDelayMs: Int,
    val maxAudioWriteMs: Int,
    val outputDroppedBytes: Long,
    val nackRequests: Long,
    val nackRecoveries: Long,
)

internal class InvalidAudioConfigException(message: String) : IllegalArgumentException(message)

internal fun validateAudioConfig(audio: AudioConfig) {
    if (audio.sampleFormat != "s16le") {
        throw InvalidAudioConfigException("Unsupported audio format ${audio.sampleFormat}")
    }
    if (audio.sampleRate !in MIN_SAMPLE_RATE..MAX_SAMPLE_RATE) {
        throw InvalidAudioConfigException("Unsupported sample rate ${audio.sampleRate}")
    }
    if (audio.channels !in 1..2) {
        throw InvalidAudioConfigException("Unsupported channel count ${audio.channels}")
    }
    if (audio.packetMs !in SUPPORTED_PACKET_DURATIONS_MS) {
        throw InvalidAudioConfigException("Unsupported packet duration ${audio.packetMs}")
    }
    if (audio.payloadType !in RTP_DYNAMIC_PAYLOAD_TYPES) {
        throw InvalidAudioConfigException("Invalid RTP payload type ${audio.payloadType}")
    }
    if (audio.ssrc !in 0..UINT32_MAX) {
        throw InvalidAudioConfigException("Invalid RTP SSRC ${audio.ssrc}")
    }
}

internal fun expectedPayloadBytes(audio: AudioConfig): Int {
    val frames = audio.sampleRate.toLong() * audio.packetMs / 1_000
    val bytes = frames * audio.channels * PCM_16_BYTES_PER_SAMPLE
    if (frames <= 0 || bytes !in 1..MAX_AUDIO_PACKET_BYTES.toLong()) {
        throw InvalidAudioConfigException("Invalid RTP audio packet size")
    }
    return bytes.toInt()
}

internal fun missingPacketWaitMs(packetMs: Int, queuedFrames: Long, sampleRate: Int): Long {
    val queuedMs = queuedFrames * 1_000 / sampleRate
    val reserveMs = maxOf(MIN_AUDIO_QUEUE_RESERVE_MS, packetMs * 2).toLong()
    return (queuedMs - reserveMs).coerceIn(packetMs.toLong(), MAX_MISSING_PACKET_WAIT_MS)
}

internal fun canWaitForMissingPacket(queuedFrames: Long, sampleRate: Int): Boolean {
    val reserveFrames = sampleRate.toLong() * MIN_AUDIO_QUEUE_RESERVE_MS / 1_000
    return queuedFrames > reserveFrames
}

internal fun packetsForDuration(durationMs: Int, packetMs: Int): Int =
    ((durationMs + packetMs - 1) / packetMs).coerceAtLeast(MIN_TARGET_PACKETS)

internal fun playbackTargetFrames(
    targetPacketCount: Int,
    framesPerPacket: Int,
    outputBufferFrames: Int,
): Long = maxOf(
    targetPacketCount.toLong() * framesPerPacket,
    outputBufferFrames.toLong(),
)

internal fun nextPowerOfTwo(value: Int): Int {
    require(value > 0)
    return Integer.highestOneBit(value - 1).coerceAtLeast(1) shl 1
}

internal data class PlaybackBufferPolicy(
    val initialBufferMs: Int,
    val minBufferMs: Int,
    val maxBufferMs: Int,
    val outputBufferMs: Int,
    val backlogMarginMs: Int,
    val directWrite: Boolean,
)

internal fun playbackBufferPolicy(playbackMode: PlaybackMode): PlaybackBufferPolicy =
    when (playbackMode) {
        PlaybackMode.Immediate -> PlaybackBufferPolicy(
            initialBufferMs = DIRECT_PACKET_BUFFER_MS,
            minBufferMs = DIRECT_PACKET_BUFFER_MS,
            maxBufferMs = DIRECT_PACKET_BUFFER_MS,
            outputBufferMs = DIRECT_PACKET_BUFFER_MS,
            backlogMarginMs = DIRECT_PACKET_BUFFER_MS,
            directWrite = true,
        )
        PlaybackMode.Adaptive -> PlaybackBufferPolicy(
            initialBufferMs = AdaptiveJitterController.INITIAL_ADAPTIVE_BUFFER_MS,
            minBufferMs = AdaptiveJitterController.MIN_ADAPTIVE_BUFFER_MS,
            maxBufferMs = MAX_BUFFER_MS,
            outputBufferMs = ADAPTIVE_OUTPUT_BUFFER_MS,
            backlogMarginMs = ADAPTIVE_BACKLOG_MARGIN_MS,
            directWrite = false,
        )
    }

internal const val MAX_BUFFER_MS = 450
internal const val ADAPTIVE_OUTPUT_BUFFER_MS = 60
internal const val STATS_UPDATE_INTERVAL_MS = 500
internal const val UNDERRUN_CHECK_INTERVAL_MS = 100
internal const val STREAM_TIMEOUT_MS = 3_000
internal const val MAX_RTP_HEADER_BYTES = 512
internal const val PACKET_QUEUE_CAPACITY = 256
internal const val PACKET_POOL_HEADROOM = 4
internal const val UDP_RECEIVE_BUFFER_BYTES = 256 * 1024
internal const val PCM_16_BYTES_PER_SAMPLE = 2
internal const val STORED_PACKET_MULTIPLIER = 4

private const val MIN_AUDIO_QUEUE_RESERVE_MS = 60
private const val MAX_MISSING_PACKET_WAIT_MS = 100L
private const val DIRECT_PACKET_BUFFER_MS = 1
private const val ADAPTIVE_BACKLOG_MARGIN_MS = 100
private const val UINT32_MAX = 0xFFFF_FFFFL
private const val MIN_SAMPLE_RATE = 8_000
private const val MAX_SAMPLE_RATE = 192_000
private const val MAX_AUDIO_PACKET_BYTES = 64 * 1024
private const val MIN_TARGET_PACKETS = 1
private val SUPPORTED_PACKET_DURATIONS_MS = setOf(5, 10, 20)
private val RTP_DYNAMIC_PAYLOAD_TYPES = 96..127
