package com.lanpulse.mobile.android

import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.media.session.PlaybackState as MediaPlaybackState
import com.lanpulse.mobile.MobileStrings
import com.lanpulse.mobile.PlaybackState

internal fun shouldStopForAudioFocusChange(focusChange: Int): Boolean =
    focusChange == AudioManager.AUDIOFOCUS_LOSS

internal fun shouldStopForOutputDeviceRemoval(deviceType: Int): Boolean = when (deviceType) {
    AudioDeviceInfo.TYPE_WIRED_HEADSET,
    AudioDeviceInfo.TYPE_WIRED_HEADPHONES,
    AudioDeviceInfo.TYPE_BLUETOOTH_A2DP,
    AudioDeviceInfo.TYPE_USB_HEADSET,
    AudioDeviceInfo.TYPE_HEARING_AID,
    AudioDeviceInfo.TYPE_BLE_HEADSET,
    AudioDeviceInfo.TYPE_BLE_SPEAKER,
    AudioDeviceInfo.TYPE_BLE_BROADCAST
    -> true
    else -> false
}

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

internal fun reconnectStatusMessage(networkAvailable: Boolean, strings: MobileStrings): String =
    if (networkAvailable) {
        strings.reconnectingInThreeSeconds
    } else {
        strings.waitingForNetwork
    }
