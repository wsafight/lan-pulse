package com.lanpulse.mobile.android

import android.media.AudioManager
import android.media.session.PlaybackState as MediaPlaybackState
import com.lanpulse.mobile.MobileStrings
import com.lanpulse.mobile.PlaybackState

internal fun shouldStopForAudioFocusChange(focusChange: Int): Boolean =
    focusChange == AudioManager.AUDIOFOCUS_LOSS ||
        focusChange == AudioManager.AUDIOFOCUS_LOSS_TRANSIENT

internal fun shouldStopForOutputDeviceRemoval(removedOutputDeviceCount: Int): Boolean =
    removedOutputDeviceCount > 0

internal fun mediaSessionStateFor(playbackState: PlaybackState): Int = when (playbackState) {
    PlaybackState.Idle -> MediaPlaybackState.STATE_STOPPED
    is PlaybackState.Connecting -> MediaPlaybackState.STATE_CONNECTING
    is PlaybackState.Playing -> MediaPlaybackState.STATE_PLAYING
    is PlaybackState.Reconnecting -> MediaPlaybackState.STATE_BUFFERING
    is PlaybackState.Failed -> MediaPlaybackState.STATE_ERROR
}

internal fun playbackFailureMessage(errorMessage: String?, strings: MobileStrings): String =
    when (errorMessage) {
        CONTROL_ERROR_INVALID_PIN -> strings.invalidPin
        CONTROL_ERROR_PIN_EXPIRED -> strings.pinExpired
        CONTROL_ERROR_DEVICE_BUSY -> strings.anotherDeviceConnected
        CONTROL_ERROR_PROTOCOL_INCOMPATIBLE -> strings.protocolIncompatible
        CONTROL_ERROR_TOO_MANY_PAIRING_ATTEMPTS -> strings.tooManyPairingAttempts
        else -> strings.desktopUnreachable
    }
