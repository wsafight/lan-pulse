package com.lanpulse.mobile

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

class ProtocolTest {
    @Test
    fun playbackModeDefaultsToAdaptiveForMissingOrUnknownStoredValues() {
        assertEquals(PlaybackMode.Immediate, PlaybackMode.fromStorageValue("immediate"))
        assertEquals(PlaybackMode.Adaptive, PlaybackMode.fromStorageValue("adaptive"))
        assertEquals(PlaybackMode.Adaptive, PlaybackMode.fromStorageValue(null))
        assertEquals(PlaybackMode.Adaptive, PlaybackMode.fromStorageValue("future-mode"))
    }

    @Test
    fun serializesStableClientAndSessionIds() {
        val connectPayload = Json.parseToJsonElement(
            lanPulseJson.encodeToString(
                ConnectRequest.serializer(),
                ConnectRequest(
                    pin = "123456",
                    udpPort = 5504,
                    clientId = "android-installation-1",
                    deviceName = "Phone",
                    sessionId = "session-2",
                ),
            ),
        ).jsonObject

        assertEquals(5504, connectPayload.getValue("udp_port").jsonPrimitive.int)
        assertEquals(
            "android-installation-1",
            connectPayload.getValue("client_id").jsonPrimitive.content,
        )
        assertEquals("Phone", connectPayload.getValue("device_name").jsonPrimitive.content)
        assertEquals("session-2", connectPayload.getValue("session_id").jsonPrimitive.content)
        assertEquals(1, connectPayload.getValue("protocol_version").jsonPrimitive.int)
        assertEquals(
            1,
            connectPayload.getValue("min_supported_protocol_version").jsonPrimitive.int,
        )
        assertTrue(connectPayload.getValue("capabilities").toString().contains("rtp-unicast"))
        assertTrue(connectPayload.getValue("capabilities").toString().contains("rtp-nack-v1"))

        val disconnectPayload = Json.parseToJsonElement(
            lanPulseJson.encodeToString(
                DisconnectRequest.serializer(),
                DisconnectRequest(pin = "123456", sessionId = "session-2"),
            ),
        ).jsonObject
        assertEquals("session-2", disconnectPayload.getValue("session_id").jsonPrimitive.content)

        val heartbeatPayload = Json.parseToJsonElement(
            lanPulseJson.encodeToString(
                HeartbeatRequest.serializer(),
                HeartbeatRequest(pin = "123456", sessionId = "session-2"),
            ),
        ).jsonObject
        assertEquals("session-2", heartbeatPayload.getValue("session_id").jsonPrimitive.content)
    }

    @Test
    fun decodesSessionIdFromConnectResponse() {
        val response = Json.decodeFromString<ConnectResponse>(
            """
                {
                  "ok":true,
                  "message":"connected Phone",
                  "session_id":"session-2",
                  "media":null
                }
            """.trimIndent(),
        )

        assertEquals("session-2", response.sessionId)
        assertEquals(1, response.protocolVersion)
        assertEquals(1, response.minSupportedProtocolVersion)
        assertTrue(response.capabilities.contains("session-id"))
        assertFalse(response.capabilities.contains("rtp-nack-v1"))
        assertTrue(response.isProtocolCompatible())
    }

    @Test
    fun decodesDesktopDiscoveryResponse() {
        val payload = """
            {
              "type":"lanpulse.desktop.v1",
              "name":"Studio PC",
              "control_url":"http://192.168.1.20:4100",
              "control_port":4100,
              "pin_required":true,
              "protocol_version":1,
              "min_supported_protocol_version":1,
              "capabilities":["pcm-s16le","rtp-unicast"],
              "audio":{
                "sample_rate":48000,
                "channels":2,
                "sample_format":"s16le",
                "packet_ms":5,
                "payload_type":96,
                "ssrc":42
              }
            }
        """.trimIndent()

        val response = Json.decodeFromString<DiscoveryResponse>(payload)

        assertEquals("Studio PC", response.name)
        assertEquals(5, response.audio.packetMs)
        assertEquals("http://192.168.1.20:4100", response.controlUrl)
        assertEquals(1, response.protocolVersion)
        assertTrue(response.isProtocolCompatible())
    }

    @Test
    fun rejectsIncompatibleProtocolVersions() {
        assertTrue(protocolVersionsAreCompatible(1, 1))
        assertFalse(protocolVersionsAreCompatible(0, 1))
        assertFalse(protocolVersionsAreCompatible(1, 2))
        assertTrue(protocolCapabilitiesAreCompatible(emptyList()))
        assertTrue(protocolCapabilitiesAreCompatible(listOf("rtp-unicast")))
        assertFalse(protocolCapabilitiesAreCompatible(listOf("unsupported-media")))

        val response = Json.decodeFromString<ConnectResponse>(
            """
                {
                  "ok":false,
                  "message":"protocol version is not compatible",
                  "protocol_version":1,
                  "min_supported_protocol_version":2,
                  "media":null
                }
            """.trimIndent(),
        )

        assertFalse(response.isProtocolCompatible())
    }

    @Test
    fun rejectsResponsesWithoutRequiredMediaCapability() {
        val discovery = Json.decodeFromString<DiscoveryResponse>(
            """
                {
                  "type":"lanpulse.desktop.v1",
                  "name":"Studio PC",
                  "control_url":"http://192.168.1.20:4100",
                  "control_port":4100,
                  "pin_required":true,
                  "protocol_version":1,
                  "min_supported_protocol_version":1,
                  "capabilities":["unsupported-media"],
                  "audio":{
                    "sample_rate":48000,
                    "channels":2,
                    "sample_format":"s16le",
                    "packet_ms":5,
                    "payload_type":96,
                    "ssrc":42
                  }
                }
            """.trimIndent(),
        )
        val connect = Json.decodeFromString<ConnectResponse>(
            """
                {
                  "ok":true,
                  "message":"connected Phone",
                  "session_id":"session-2",
                  "protocol_version":1,
                  "min_supported_protocol_version":1,
                  "capabilities":["unsupported-media"],
                  "media":null
                }
            """.trimIndent(),
        )

        assertFalse(discovery.isProtocolCompatible())
        assertFalse(connect.isProtocolCompatible())
    }

    @Test
    fun validatesPairingInput() {
        assertTrue(isValidPin("123456"))
        assertFalse(isValidPin("12345"))
        assertFalse(isValidPin("12345a"))
        assertNotNull(manualEndpoint("192.168.1.20:4100"))
        assertEquals(
            "http://192.168.1.20:4100",
            manualEndpoint("192.168.1.20:4100")?.controlUrl,
        )
        assertNull(manualEndpoint("192.168.1.20"))
        assertNull(manualEndpoint("https://192.168.1.20:4100"))
    }

    @Test
    fun parsesDesktopPairingCode() {
        val pairing = parsePairingCode(
            "lanpulse://pair?url=http%3A%2F%2F192.168.1.20%3A4100&pin=123456",
        )

        assertNotNull(pairing)
        assertEquals("http://192.168.1.20:4100", pairing.controlUrl)
        assertEquals("123456", pairing.pin)
    }

    @Test
    fun rejectsNonLanOrMalformedPairingCodes() {
        assertNull(parsePairingCode("https://example.com"))
        assertNull(
            parsePairingCode(
                "lanpulse://pair?url=http%3A%2F%2F8.8.8.8%3A4100&pin=123456",
            ),
        )
        assertNull(
            parsePairingCode(
                "lanpulse://pair?url=http%3A%2F%2F192.168.1.20%3A4100&pin=12345",
            ),
        )
    }

    @Test
    fun rejectsPairingCodesWithFragmentsDuplicatesOrInvalidEncoding() {
        assertNull(
            parsePairingCode(
                "lanpulse://pair?url=http%3A%2F%2F192.168.1.20%3A4100&pin=123456#frag",
            ),
        )
        assertNull(
            parsePairingCode(
                "lanpulse://pair?url=http%3A%2F%2F192.168.1.20%3A4100&pin=123456&pin=654321",
            ),
        )
        assertNull(
            parsePairingCode(
                "lanpulse://pair?url=http%3A%2F%2F192.168.1.20%3A4100&pin=123456&bad",
            ),
        )
        assertNull(
            parsePairingCode(
                "lanpulse://pair?url=http%3A%2F%2F192.168.1.20%3A4100&pin=12%ZZ56",
            ),
        )
    }

    @Test
    fun acceptsPrivateAndLoopbackPairingHosts() {
        val hosts = listOf("10.0.0.2", "127.0.0.1", "169.254.1.2", "172.16.1.2", "172.31.1.2")

        hosts.forEach { host ->
            val pairing = parsePairingCode(
                "lanpulse://pair?url=http%3A%2F%2F$host%3A4100&pin=123456",
            )

            assertNotNull(pairing, "expected $host to be accepted")
            assertEquals("http://$host:4100", pairing.controlUrl)
        }
    }

    @Test
    fun manualEndpointRejectsUnsafeOrIncompleteAuthorities() {
        assertNull(manualEndpoint(""))
        assertNull(manualEndpoint("http://192.168.1.20:4100/path"))
        assertNull(manualEndpoint("192.168.1.20:0"))
        assertNull(manualEndpoint("192.168.1.20:65536"))
        assertNull(manualEndpoint(":4100"))

        assertEquals(
            "http://192.168.1.20:4100",
            manualEndpoint(" 192.168.1.20:4100/ ")?.controlUrl,
        )
    }
}
