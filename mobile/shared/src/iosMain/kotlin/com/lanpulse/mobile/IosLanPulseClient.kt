package com.lanpulse.mobile

import androidx.compose.runtime.remember
import androidx.compose.ui.window.ComposeUIViewController
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.suspendCancellableCoroutine
import platform.UIKit.UIViewController
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

class IosDesktopEndpoint(
    val name: String,
    val controlUrl: String,
    val sampleRate: Int,
    val channels: Int,
    val sampleFormat: String,
    val packetMs: Int,
    val payloadType: Int,
    val ssrc: Long,
    val hasAudio: Boolean,
)

interface IosPlaybackObserver {
    fun onIdle()

    fun onConnecting(desktopName: String)

    fun onPlaying(
        desktopName: String,
        packetsReceived: Long,
        packetsLost: Long,
        bufferMs: Int,
    )

    fun onReconnecting(desktopName: String, reason: String)

    fun onFailed(message: String)

    fun onPairingCode(value: String)

    fun onPairingScanFailed(message: String)
}

interface IosLanPulseBackend {
    val initialLanguageCode: String

    fun setObserver(observer: IosPlaybackObserver)

    fun discover(completion: (List<IosDesktopEndpoint>?, String?) -> Unit)

    fun connect(endpoint: IosDesktopEndpoint, pin: String, languageCode: String)

    fun disconnect()

    fun scanPairingCode(languageCode: String)

    fun saveLanguage(languageCode: String)
}

private class IosLanPulseClient(
    private val backend: IosLanPulseBackend,
) : LanPulseClient, IosPlaybackObserver {
    private val mutablePlaybackState = MutableStateFlow<PlaybackState>(PlaybackState.Idle)
    private val mutablePairingScanEvents = MutableSharedFlow<PairingScanEvent>(extraBufferCapacity = 1)
    private var currentLanguage = MobileLanguage.fromCode(backend.initialLanguageCode)

    override val playbackState: StateFlow<PlaybackState> = mutablePlaybackState.asStateFlow()
    override val pairingScanEvents: SharedFlow<PairingScanEvent> =
        mutablePairingScanEvents.asSharedFlow()
    override val initialLanguage: MobileLanguage = currentLanguage

    init {
        backend.setObserver(this)
    }

    override suspend fun discover(): List<DesktopEndpoint> = suspendCancellableCoroutine { continuation ->
        backend.discover { endpoints, error ->
            if (!continuation.isActive) return@discover
            when {
                error != null -> continuation.resumeWithException(IllegalStateException(error))
                endpoints != null -> continuation.resume(endpoints.map(IosDesktopEndpoint::toCommon))
                else -> continuation.resume(emptyList())
            }
        }
    }

    override fun connect(
        endpoint: DesktopEndpoint,
        pin: String,
        language: MobileLanguage,
        playbackMode: PlaybackMode,
    ) {
        currentLanguage = language
        backend.connect(endpoint.toIos(), pin, language.code)
    }

    override fun disconnect() {
        backend.disconnect()
    }

    override fun scanPairingCode(language: MobileLanguage) {
        currentLanguage = language
        backend.scanPairingCode(language.code)
    }

    override fun saveLanguage(language: MobileLanguage) {
        currentLanguage = language
        backend.saveLanguage(language.code)
    }

    override fun onIdle() {
        mutablePlaybackState.value = PlaybackState.Idle
    }

    override fun onConnecting(desktopName: String) {
        mutablePlaybackState.value = PlaybackState.Connecting(desktopName)
    }

    override fun onPlaying(
        desktopName: String,
        packetsReceived: Long,
        packetsLost: Long,
        bufferMs: Int,
    ) {
        mutablePlaybackState.value = PlaybackState.Playing(
            desktopName,
            packetsReceived,
            packetsLost,
            bufferMs,
        )
    }

    override fun onReconnecting(desktopName: String, reason: String) {
        mutablePlaybackState.value = PlaybackState.Reconnecting(desktopName, reason)
    }

    override fun onFailed(message: String) {
        mutablePlaybackState.value = PlaybackState.Failed(message)
    }

    override fun onPairingCode(value: String) {
        val pairing = parsePairingCode(value)
        mutablePairingScanEvents.tryEmit(
            pairing?.let(PairingScanEvent::Found)
                ?: PairingScanEvent.Failed(currentLanguage.strings().notPairingCode),
        )
    }

    override fun onPairingScanFailed(message: String) {
        mutablePairingScanEvents.tryEmit(PairingScanEvent.Failed(message))
    }
}

fun MainViewController(backend: IosLanPulseBackend): UIViewController =
    ComposeUIViewController {
        val client = remember(backend) { IosLanPulseClient(backend) }
        LanPulseApp(client)
    }

private fun IosDesktopEndpoint.toCommon(): DesktopEndpoint = DesktopEndpoint(
    name = name,
    controlUrl = controlUrl,
    audio = if (hasAudio) {
        AudioConfig(
            sampleRate = sampleRate,
            channels = channels,
            sampleFormat = sampleFormat,
            packetMs = packetMs,
            payloadType = payloadType,
            ssrc = ssrc,
        )
    } else {
        null
    },
)

private fun DesktopEndpoint.toIos(): IosDesktopEndpoint {
    val config = audio
    return IosDesktopEndpoint(
        name = name,
        controlUrl = controlUrl,
        sampleRate = config?.sampleRate ?: 0,
        channels = config?.channels ?: 0,
        sampleFormat = config?.sampleFormat.orEmpty(),
        packetMs = config?.packetMs ?: 0,
        payloadType = config?.payloadType ?: 0,
        ssrc = config?.ssrc ?: 0,
        hasAudio = config != null,
    )
}
