package com.lanpulse.mobile.android

import android.media.AudioManager
import android.media.session.PlaybackState as MediaPlaybackState
import com.lanpulse.mobile.MobileLanguage
import com.lanpulse.mobile.PlaybackState
import java.net.HttpURLConnection
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class AndroidPlaybackLifecycleTest {
    @Test
    fun stopsForPermanentOrTransientAudioFocusLoss() {
        assertTrue(shouldStopForAudioFocusChange(AudioManager.AUDIOFOCUS_LOSS))
        assertTrue(shouldStopForAudioFocusChange(AudioManager.AUDIOFOCUS_LOSS_TRANSIENT))
        assertFalse(shouldStopForAudioFocusChange(AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK))
        assertFalse(shouldStopForAudioFocusChange(AudioManager.AUDIOFOCUS_GAIN))
    }

    @Test
    fun stopsWhenAnyOutputDeviceIsRemoved() {
        assertFalse(shouldStopForOutputDeviceRemoval(0))
        assertTrue(shouldStopForOutputDeviceRemoval(1))
    }

    @Test
    fun mapsPlaybackStateToMediaSessionState() {
        assertEquals(MediaPlaybackState.STATE_STOPPED, mediaSessionStateFor(PlaybackState.Idle))
        assertEquals(
            MediaPlaybackState.STATE_CONNECTING,
            mediaSessionStateFor(PlaybackState.Connecting("Studio")),
        )
        assertEquals(
            MediaPlaybackState.STATE_PLAYING,
            mediaSessionStateFor(
                PlaybackState.Playing(
                    desktopName = "Studio",
                    packetsReceived = 10,
                    packetsLost = 0,
                    bufferMs = 15,
                ),
            ),
        )
        assertEquals(
            MediaPlaybackState.STATE_BUFFERING,
            mediaSessionStateFor(PlaybackState.Reconnecting("Studio", "offline")),
        )
        assertEquals(
            MediaPlaybackState.STATE_ERROR,
            mediaSessionStateFor(PlaybackState.Failed("offline")),
        )
    }

    @Test
    fun mapsControlHttpErrorsToStableMessages() {
        assertEquals(
            CONTROL_ERROR_PIN_EXPIRED,
            controlErrorMessage(
                HttpURLConnection.HTTP_UNAUTHORIZED,
                """{"ok":false,"message":"pin expired","media":null}""",
            ),
        )
        assertEquals(
            CONTROL_ERROR_INVALID_PIN,
            controlErrorMessage(HttpURLConnection.HTTP_UNAUTHORIZED, "access denied"),
        )
        assertEquals(CONTROL_ERROR_DEVICE_BUSY, controlErrorMessage(HttpURLConnection.HTTP_CONFLICT, ""))
        assertEquals(CONTROL_ERROR_PROTOCOL_INCOMPATIBLE, controlErrorMessage(426, ""))
        assertEquals(CONTROL_ERROR_TOO_MANY_PAIRING_ATTEMPTS, controlErrorMessage(429, ""))
    }

    @Test
    fun localizesPlaybackControlFailures() {
        val strings = MobileLanguage.En.strings()

        assertEquals(strings.invalidPin, playbackFailureMessage(CONTROL_ERROR_INVALID_PIN, strings))
        assertEquals(strings.pinExpired, playbackFailureMessage(CONTROL_ERROR_PIN_EXPIRED, strings))
        assertEquals(
            strings.anotherDeviceConnected,
            playbackFailureMessage(CONTROL_ERROR_DEVICE_BUSY, strings),
        )
        assertEquals(
            strings.protocolIncompatible,
            playbackFailureMessage(CONTROL_ERROR_PROTOCOL_INCOMPATIBLE, strings),
        )
        assertEquals(
            strings.tooManyPairingAttempts,
            playbackFailureMessage(CONTROL_ERROR_TOO_MANY_PAIRING_ATTEMPTS, strings),
        )
        assertEquals(strings.desktopUnreachable, playbackFailureMessage("other", strings))
    }
}
