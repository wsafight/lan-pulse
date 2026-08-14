package com.lanpulse.mobile

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

enum class DiscoveryState {
    Idle,
    Searching,
    Complete,
}

data class MobileUiState(
    val language: MobileLanguage = MobileLanguage.En,
    val discoveryState: DiscoveryState = DiscoveryState.Idle,
    val desktops: List<DesktopEndpoint> = emptyList(),
    val selectedDesktopId: String? = null,
    val manualUrl: String = "",
    val pin: String = "",
    val playback: PlaybackState = PlaybackState.Idle,
    val error: String? = null,
)

class LanPulseController(
    private val client: LanPulseClient,
    private val scope: CoroutineScope,
) {
    private val mutableState = MutableStateFlow(MobileUiState(language = client.initialLanguage))
    private var discoveryJob: Job? = null

    val state: StateFlow<MobileUiState> = mutableState.asStateFlow()

    init {
        scope.launch {
            client.playbackState.collect { playback ->
                mutableState.update { current ->
                    current.copy(
                        playback = playback,
                        error = (playback as? PlaybackState.Failed)?.message ?: current.error,
                    )
                }
            }
        }
        scope.launch {
            client.pairingScanEvents.collect { event ->
                when (event) {
                    is PairingScanEvent.Found -> applyPairingCode(event.pairing)
                    is PairingScanEvent.Failed -> mutableState.update {
                        it.copy(error = event.message)
                    }
                }
            }
        }
    }

    fun discover() {
        if (discoveryJob?.isActive == true) return
        discoveryJob = scope.launch {
            mutableState.update { it.copy(discoveryState = DiscoveryState.Searching, error = null) }
            runCatching { client.discover() }
                .onSuccess { desktops ->
                    mutableState.update { current ->
                        current.copy(
                            discoveryState = DiscoveryState.Complete,
                            desktops = desktops,
                            selectedDesktopId = current.selectedDesktopId
                                ?.takeIf { id -> desktops.any { it.id == id } }
                                ?: desktops.firstOrNull()?.id,
                        )
                    }
                }
                .onFailure { error ->
                    mutableState.update {
                        it.copy(
                            discoveryState = DiscoveryState.Complete,
                            error = error.message ?: it.language.strings().discoveryFailed,
                        )
                    }
                }
        }
    }

    fun selectDesktop(id: String) {
        mutableState.update {
            it.copy(selectedDesktopId = id, manualUrl = "", error = null)
        }
    }

    fun updateManualUrl(value: String) {
        mutableState.update {
            it.copy(
                manualUrl = value,
                selectedDesktopId = if (value.isBlank()) it.selectedDesktopId else null,
                error = null,
            )
        }
    }

    fun updatePin(value: String) {
        val digits = value.filter(Char::isDigit).take(PIN_LENGTH)
        mutableState.update { it.copy(pin = digits, error = null) }
    }

    fun connect() {
        val current = mutableState.value
        val strings = current.language.strings()
        if (!isValidPin(current.pin)) {
            mutableState.update { it.copy(error = strings.enterPin) }
            return
        }

        val endpoint = manualEndpoint(current.manualUrl)
            ?.copy(name = strings.manualComputer)
            ?: current.desktops.firstOrNull { it.id == current.selectedDesktopId }
        if (endpoint == null) {
            mutableState.update { it.copy(error = strings.selectComputer) }
            return
        }

        runCatching { client.connect(endpoint, current.pin, current.language) }
            .onFailure { error ->
                mutableState.update { it.copy(error = error.message ?: strings.unableToConnect) }
            }
    }

    fun disconnect() {
        client.disconnect()
    }

    fun scanPairingCode() {
        val language = mutableState.value.language
        mutableState.update { it.copy(error = null) }
        runCatching { client.scanPairingCode(language) }
            .onFailure { error ->
                mutableState.update {
                    it.copy(error = error.message ?: language.strings().unableToOpenCamera)
                }
            }
    }

    fun selectLanguage(language: MobileLanguage) {
        mutableState.update { it.copy(language = language, error = null) }
        client.saveLanguage(language)
    }

    fun clearError() {
        mutableState.update { it.copy(error = null) }
    }

    private fun applyPairingCode(pairing: PairingCode) {
        mutableState.update {
            it.copy(
                selectedDesktopId = null,
                manualUrl = pairing.controlUrl,
                pin = pairing.pin,
                error = null,
            )
        }
    }
}

internal const val PIN_LENGTH = 6

internal fun isValidPin(pin: String): Boolean =
    pin.length == PIN_LENGTH && pin.all(Char::isDigit)

internal fun manualEndpoint(value: String): DesktopEndpoint? {
    val trimmed = value.trim().trimEnd('/')
    if (trimmed.isEmpty()) return null
    val normalized = if (trimmed.startsWith("http://")) trimmed else "http://$trimmed"
    val authority = normalized.removePrefix("http://")
    if ('/' in authority || ':' !in authority) return null
    val host = authority.substringBeforeLast(':')
    val port = authority.substringAfterLast(':').toIntOrNull()
    if (host.isBlank() || port == null || port !in 1..65535) return null
    return DesktopEndpoint(name = "Manual computer", controlUrl = normalized, audio = null)
}
