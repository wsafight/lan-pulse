package com.lanpulse.mobile.android

import android.content.Context
import android.content.Intent
import android.os.Build
import com.lanpulse.mobile.DesktopEndpoint
import com.lanpulse.mobile.LanPulseClient
import com.lanpulse.mobile.MobileLanguage
import com.lanpulse.mobile.PairingScanEvent
import com.lanpulse.mobile.PlaybackMode
import com.lanpulse.mobile.PlaybackState
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

class AndroidLanPulseClient(
    context: Context,
    private val onForegroundPlaybackRequested: () -> Unit = {},
) : LanPulseClient {
    private val appContext = context.applicationContext.also(AndroidDiagnosticLog::initialize)
    private val clientId = AndroidClientIdentityStore.loadOrCreate(appContext)

    override val playbackState: StateFlow<PlaybackState> = AndroidPlaybackSession.state
    override val initialLanguage: MobileLanguage = AndroidLanguageStore.load(appContext)
    override val initialPlaybackMode: PlaybackMode = AndroidPlaybackModeStore.load(appContext)
    override val supportsPlaybackMode: Boolean = true
    override val pairingScanEvents: Flow<PairingScanEvent> = AndroidPairingScanner.events

    override suspend fun discover(): List<DesktopEndpoint> = UdpDiscoveryClient.discover()

    override fun connect(
        endpoint: DesktopEndpoint,
        pin: String,
        language: MobileLanguage,
        playbackMode: PlaybackMode,
    ) {
        AndroidDiagnosticLog.event(
            "connect_requested",
            "desktop=${endpoint.name} control_url=${endpoint.controlUrl} " +
                "playback_mode=${playbackMode.storageValue}",
        )
        onForegroundPlaybackRequested()
        val intent = Intent(appContext, LanPulsePlaybackService::class.java).apply {
            action = LanPulsePlaybackService.ACTION_CONNECT
            putExtra(LanPulsePlaybackService.EXTRA_DESKTOP_NAME, endpoint.name)
            putExtra(LanPulsePlaybackService.EXTRA_CONTROL_URL, endpoint.controlUrl)
            putExtra(LanPulsePlaybackService.EXTRA_PIN, pin)
            putExtra(LanPulsePlaybackService.EXTRA_CLIENT_ID, clientId)
            putExtra(LanPulsePlaybackService.EXTRA_LANGUAGE, language.code)
            putExtra(LanPulsePlaybackService.EXTRA_PLAYBACK_MODE, playbackMode.storageValue)
        }
        runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                appContext.startForegroundService(intent)
            } else {
                appContext.startService(intent)
            }
        }.onFailure { error ->
            AndroidDiagnosticLog.error("foreground_service_start_failed", error)
            AndroidPlaybackSession.update(
                PlaybackState.Failed(error.message ?: language.strings().unableToConnect),
            )
        }
    }

    override fun disconnect() {
        AndroidDiagnosticLog.event("disconnect_requested")
        val intent = Intent(appContext, LanPulsePlaybackService::class.java).apply {
            action = LanPulsePlaybackService.ACTION_DISCONNECT
        }
        runCatching { appContext.startService(intent) }
            .onFailure {
                AndroidDiagnosticLog.error("disconnect_service_start_failed", it)
                AndroidPlaybackSession.update(PlaybackState.Idle)
                appContext.stopService(intent)
            }
    }

    override fun scanPairingCode(language: MobileLanguage) {
        AndroidDiagnosticLog.event("pairing_scanner_requested")
        val intent = Intent(appContext, LanPulseQrScannerActivity::class.java)
            .putExtra(LanPulseQrScannerActivity.EXTRA_LANGUAGE, language.code)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        runCatching { appContext.startActivity(intent) }
            .onFailure { error ->
                AndroidDiagnosticLog.error("pairing_scanner_start_failed", error)
                AndroidPairingScanner.publish(
                    PairingScanEvent.Failed(
                        error.message ?: language.strings().unableToOpenCamera,
                    ),
                )
            }
    }

    override fun saveLanguage(language: MobileLanguage) {
        AndroidLanguageStore.save(appContext, language)
    }

    override fun savePlaybackMode(playbackMode: PlaybackMode) {
        AndroidPlaybackModeStore.save(appContext, playbackMode)
    }
}

internal object AndroidLanguageStore {
    private const val PREFERENCES = "lanpulse_mobile"
    private const val LANGUAGE = "language"

    fun load(context: Context): MobileLanguage = MobileLanguage.fromCode(
        context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
            .getString(LANGUAGE, null),
    )

    fun save(context: Context, language: MobileLanguage) {
        context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
            .edit()
            .putString(LANGUAGE, language.code)
            .apply()
    }
}

internal object AndroidPlaybackModeStore {
    private const val PREFERENCES = "lanpulse_mobile"
    private const val PLAYBACK_MODE = "playback_mode"

    fun load(context: Context): PlaybackMode = PlaybackMode.fromStorageValue(
        context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
            .getString(PLAYBACK_MODE, null),
    )

    fun save(context: Context, playbackMode: PlaybackMode) {
        context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
            .edit()
            .putString(PLAYBACK_MODE, playbackMode.storageValue)
            .apply()
    }
}

internal object AndroidClientIdentityStore {
    private const val PREFERENCES = "lanpulse_mobile"
    private const val CLIENT_ID = "client_id"

    fun loadOrCreate(context: Context): String {
        val preferences = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
        preferences.getString(CLIENT_ID, null)?.takeIf(String::isNotBlank)?.let { return it }
        val clientId = java.util.UUID.randomUUID().toString()
        preferences.edit().putString(CLIENT_ID, clientId).commit()
        return preferences.getString(CLIENT_ID, clientId) ?: clientId
    }
}

internal object AndroidPlaybackSession {
    private val mutableState = MutableStateFlow<PlaybackState>(PlaybackState.Idle)

    val state: StateFlow<PlaybackState> = mutableState.asStateFlow()

    fun update(state: PlaybackState) {
        mutableState.value = state
    }
}
