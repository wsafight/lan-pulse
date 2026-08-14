package com.lanpulse.mobile

import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.emptyFlow

sealed interface PlaybackState {
    data object Idle : PlaybackState
    data class Connecting(val desktopName: String) : PlaybackState
    data class Playing(
        val desktopName: String,
        val packetsReceived: Long,
        val packetsLost: Long,
        val bufferMs: Int,
    ) : PlaybackState
    data class Reconnecting(val desktopName: String, val reason: String) : PlaybackState
    data class Failed(val message: String) : PlaybackState
}

sealed interface PairingScanEvent {
    data class Found(val pairing: PairingCode) : PairingScanEvent
    data class Failed(val message: String) : PairingScanEvent
}

interface LanPulseClient {
    val playbackState: StateFlow<PlaybackState>
    val initialLanguage: MobileLanguage
        get() = MobileLanguage.En
    val pairingScanEvents: Flow<PairingScanEvent>
        get() = emptyFlow()

    suspend fun discover(): List<DesktopEndpoint>

    fun connect(endpoint: DesktopEndpoint, pin: String, language: MobileLanguage)

    fun disconnect()

    fun scanPairingCode(language: MobileLanguage) {}

    fun saveLanguage(language: MobileLanguage) {}
}
