package com.lanpulse.mobile

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

const val LANPULSE_PROTOCOL_VERSION = 1
const val LANPULSE_MIN_SUPPORTED_PROTOCOL_VERSION = 1
const val LANPULSE_CAPABILITY_PCM_S16LE = "pcm-s16le"
const val LANPULSE_CAPABILITY_RTP_UNICAST = "rtp-unicast"
const val LANPULSE_CAPABILITY_SESSION_ID = "session-id"
const val LANPULSE_CAPABILITY_CLIENT_ID = "client-id"
const val LANPULSE_CAPABILITY_LEASE_HEARTBEAT = "lease-heartbeat"
const val LANPULSE_CAPABILITY_RTP_NACK_V1 = "rtp-nack-v1"

private val LANPULSE_LEGACY_CAPABILITIES: List<String> = listOf(
    LANPULSE_CAPABILITY_PCM_S16LE,
    LANPULSE_CAPABILITY_RTP_UNICAST,
    LANPULSE_CAPABILITY_SESSION_ID,
    LANPULSE_CAPABILITY_CLIENT_ID,
    LANPULSE_CAPABILITY_LEASE_HEARTBEAT,
)

val LANPULSE_CAPABILITIES: List<String> =
    LANPULSE_LEGACY_CAPABILITIES + LANPULSE_CAPABILITY_RTP_NACK_V1

val lanPulseJson: Json = Json {
    ignoreUnknownKeys = true
    encodeDefaults = true
}

fun protocolVersionsAreCompatible(
    peerVersion: Int,
    peerMinSupportedVersion: Int,
): Boolean =
    peerVersion >= LANPULSE_MIN_SUPPORTED_PROTOCOL_VERSION &&
        peerMinSupportedVersion <= LANPULSE_PROTOCOL_VERSION

fun protocolCapabilitiesAreCompatible(capabilities: List<String>): Boolean =
    capabilities.isEmpty() || LANPULSE_CAPABILITY_RTP_UNICAST in capabilities

@Serializable
data class AudioConfig(
    @SerialName("sample_rate") val sampleRate: Int,
    val channels: Int,
    @SerialName("sample_format") val sampleFormat: String,
    @SerialName("packet_ms") val packetMs: Int,
    @SerialName("payload_type") val payloadType: Int,
    val ssrc: Long,
)

@Serializable
data class DiscoveryResponse(
    val type: String,
    val name: String,
    @SerialName("control_url") val controlUrl: String,
    @SerialName("control_port") val controlPort: Int,
    @SerialName("pin_required") val pinRequired: Boolean,
    val audio: AudioConfig,
    @SerialName("protocol_version")
    val protocolVersion: Int = LANPULSE_PROTOCOL_VERSION,
    @SerialName("min_supported_protocol_version")
    val minSupportedProtocolVersion: Int = LANPULSE_MIN_SUPPORTED_PROTOCOL_VERSION,
    val capabilities: List<String> = LANPULSE_LEGACY_CAPABILITIES,
) {
    fun isProtocolCompatible(): Boolean =
        protocolVersionsAreCompatible(protocolVersion, minSupportedProtocolVersion) &&
            protocolCapabilitiesAreCompatible(capabilities)
}

data class DesktopEndpoint(
    val name: String,
    val controlUrl: String,
    val audio: AudioConfig?,
    val protocolVersion: Int = LANPULSE_PROTOCOL_VERSION,
    val minSupportedProtocolVersion: Int = LANPULSE_MIN_SUPPORTED_PROTOCOL_VERSION,
    val capabilities: List<String> = LANPULSE_LEGACY_CAPABILITIES,
) {
    val id: String = controlUrl.trimEnd('/')
}

@Serializable
data class ConnectRequest(
    val pin: String,
    @SerialName("udp_port") val udpPort: Int,
    @SerialName("client_id") val clientId: String,
    @SerialName("device_name") val deviceName: String,
    @SerialName("session_id") val sessionId: String? = null,
    @SerialName("protocol_version") val protocolVersion: Int = LANPULSE_PROTOCOL_VERSION,
    @SerialName("min_supported_protocol_version") val minSupportedProtocolVersion: Int =
        LANPULSE_MIN_SUPPORTED_PROTOCOL_VERSION,
    val capabilities: List<String> = LANPULSE_CAPABILITIES,
)

@Serializable
data class ConnectResponse(
    val ok: Boolean,
    val message: String,
    @SerialName("session_id") val sessionId: String? = null,
    @SerialName("protocol_version") val protocolVersion: Int = LANPULSE_PROTOCOL_VERSION,
    @SerialName("min_supported_protocol_version") val minSupportedProtocolVersion: Int =
        LANPULSE_MIN_SUPPORTED_PROTOCOL_VERSION,
    val capabilities: List<String> = LANPULSE_LEGACY_CAPABILITIES,
    val media: MediaConfig? = null,
) {
    fun isProtocolCompatible(): Boolean =
        protocolVersionsAreCompatible(protocolVersion, minSupportedProtocolVersion) &&
            protocolCapabilitiesAreCompatible(capabilities)
}

@Serializable
data class DisconnectRequest(
    val pin: String,
    @SerialName("session_id") val sessionId: String? = null,
)

@Serializable
data class HeartbeatRequest(
    val pin: String,
    @SerialName("session_id") val sessionId: String,
)

@Serializable
data class MediaConfig(
    @SerialName("target_ip") val targetIp: String,
    @SerialName("target_port") val targetPort: Int,
    val audio: AudioConfig,
)
