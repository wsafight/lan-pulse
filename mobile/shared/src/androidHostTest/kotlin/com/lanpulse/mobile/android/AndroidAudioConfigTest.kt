package com.lanpulse.mobile.android

import com.lanpulse.mobile.AudioConfig
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith

class AndroidAudioConfigTest {
    @Test
    fun computesExpectedPayloadBytesForSupportedPacketSizes() {
        assertEquals(960, expectedPayloadBytes(audio(packetMs = 5)))
        assertEquals(1_920, expectedPayloadBytes(audio(packetMs = 10)))
        assertEquals(3_840, expectedPayloadBytes(audio(packetMs = 20)))
        assertEquals(480, expectedPayloadBytes(audio(channels = 1, packetMs = 5)))
    }

    @Test
    fun acceptsValidDesktopAudioConfig() {
        validateAudioConfig(audio())
    }

    @Test
    fun rejectsUnsupportedAudioConfigFields() {
        assertInvalid(audio(sampleFormat = "f32le"))
        assertInvalid(audio(sampleRate = 7_999))
        assertInvalid(audio(sampleRate = 192_001))
        assertInvalid(audio(channels = 0))
        assertInvalid(audio(channels = 3))
        assertInvalid(audio(packetMs = 7))
        assertInvalid(audio(payloadType = 95))
        assertInvalid(audio(payloadType = 128))
        assertInvalid(audio(ssrc = -1))
        assertInvalid(audio(ssrc = 0x1_0000_0000))
    }

    @Test
    fun sizingHelpersRoundUpAndApplyMinimums() {
        assertEquals(2, packetsForDuration(durationMs = 1, packetMs = 5))
        assertEquals(3, packetsForDuration(durationMs = 11, packetMs = 5))
        assertEquals(2, nextPowerOfTwo(1))
        assertEquals(8, nextPowerOfTwo(5))
        assertEquals(5, missingPacketWaitMs(packetMs = 5, queuedFrames = 0, sampleRate = 48_000))
        assertEquals(20, missingPacketWaitMs(packetMs = 20, queuedFrames = 0, sampleRate = 48_000))
    }

    @Test
    fun expectedPayloadBytesRejectsOversizedPackets() {
        assertFailsWith<InvalidAudioConfigException> {
            expectedPayloadBytes(audio(sampleRate = 192_000, channels = 2, packetMs = 200))
        }
    }

    @Test
    fun nextPowerOfTwoRejectsNonPositiveInput() {
        assertFailsWith<IllegalArgumentException> {
            nextPowerOfTwo(0)
        }
    }

    private fun assertInvalid(audio: AudioConfig) {
        assertFailsWith<InvalidAudioConfigException> {
            validateAudioConfig(audio)
        }
    }

    private fun audio(
        sampleRate: Int = 48_000,
        channels: Int = 2,
        sampleFormat: String = "s16le",
        packetMs: Int = 5,
        payloadType: Int = 96,
        ssrc: Long = 1,
    ): AudioConfig = AudioConfig(
        sampleRate = sampleRate,
        channels = channels,
        sampleFormat = sampleFormat,
        packetMs = packetMs,
        payloadType = payloadType,
        ssrc = ssrc,
    )
}
