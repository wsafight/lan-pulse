package com.lanpulse.mobile.android

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.drawable.Icon
import android.media.AudioAttributes
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.media.session.MediaSession
import android.media.session.PlaybackState as MediaPlaybackState
import android.net.ConnectivityManager
import android.net.Network
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.IBinder
import android.os.PowerManager
import android.os.Process
import com.lanpulse.mobile.MobileLanguage
import com.lanpulse.mobile.MobileStrings
import com.lanpulse.mobile.PlaybackState as LanPulsePlaybackState
import java.net.DatagramSocket
import java.util.concurrent.Executors
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeoutOrNull

class LanPulsePlaybackService : Service() {
    private val serviceScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private val audioDispatcher = Executors.newSingleThreadExecutor { command ->
        Thread(
            {
                Process.setThreadPriority(Process.THREAD_PRIORITY_AUDIO)
                command.run()
            },
            "LanPulseAudio",
        )
    }.asCoroutineDispatcher()
    private val commandMutex = Mutex()
    private var playbackJob: Job? = null
    private var activeSession: PlaybackSession? = null
    private var notificationLanguage = MobileLanguage.En
    private var wakeLock: PowerManager.WakeLock? = null
    private var wifiLock: WifiManager.WifiLock? = null
    private var audioManager: AudioManager? = null
    private var connectivityManager: ConnectivityManager? = null
    private var audioFocusRequest: AudioFocusRequest? = null
    private var outputDeviceCallback: AudioDeviceCallback? = null
    private var networkCallback: ConnectivityManager.NetworkCallback? = null
    private var mediaSession: MediaSession? = null
    private var lastNotificationContent: String? = null
    private val networkSignal = Channel<Unit>(Channel.CONFLATED)

    override fun onCreate() {
        super.onCreate()
        notificationLanguage = AndroidLanguageStore.load(this)
        audioManager = getSystemService(Context.AUDIO_SERVICE) as AudioManager
        connectivityManager = getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        mediaSession = createMediaSession()
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_CONNECT -> {
                val session = intent.toPlaybackSession()
                if (session == null) {
                    failAndStop(notificationLanguage.strings().missingConnectionDetails)
                } else {
                    notificationLanguage = session.language
                    createNotificationChannel()
                    startAsForeground(session.desktopName, session.strings.connecting)
                    updatePlaybackState(LanPulsePlaybackState.Connecting(session.desktopName))
                    serviceScope.launch { replaceSession(session) }
                }
            }

            ACTION_DISCONNECT -> serviceScope.launch { stopSession() }
            else -> stopSelf(startId)
        }
        return START_NOT_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        playbackJob?.cancel()
        releasePlaybackResources()
        mediaSession?.release()
        mediaSession = null
        networkSignal.close()
        serviceScope.cancel()
        audioDispatcher.close()
        super.onDestroy()
    }

    private suspend fun replaceSession(session: PlaybackSession) = commandMutex.withLock {
        playbackJob?.cancelAndJoin()

        activeSession = session
        if (!requestPlaybackFocus()) {
            failAndStop(session.strings.unableToConnect)
            return@withLock
        }
        acquirePlaybackResources()
        playbackJob = serviceScope.launch(audioDispatcher) { runPlaybackLoop(session) }
    }

    private suspend fun runPlaybackLoop(session: PlaybackSession) {
        val strings = session.strings
        var lastFailure = strings.connectionLost
        var reconnectSessionId: String? = null
        try {
            while (currentCoroutineContext().isActive) {
                try {
                    playOnce(session, reconnectSessionId) { sessionId ->
                        reconnectSessionId = sessionId
                    }
                    lastFailure = strings.audioStreamEnded
                } catch (error: CancellationException) {
                    throw error
                } catch (error: ControlFailure) {
                    lastFailure = playbackFailureMessage(error.message, strings)
                    if (!error.retryable) {
                        disconnectRegisteredSession(session, reconnectSessionId)
                        reconnectSessionId = null
                        failAndStop(lastFailure)
                        return
                    }
                } catch (_: InvalidAudioConfigException) {
                    disconnectRegisteredSession(session, reconnectSessionId)
                    reconnectSessionId = null
                    failAndStop(strings.incompatibleAudioFormat)
                    return
                } catch (error: Exception) {
                    lastFailure = strings.audioPlaybackStopped
                }

                updatePlaybackState(
                    LanPulsePlaybackState.Reconnecting(session.desktopName, lastFailure),
                )
                updateNotification(
                    session.desktopName,
                    reconnectStatusMessage(isNetworkAvailable(), strings),
                )
                waitBeforeReconnect()
                updatePlaybackState(LanPulsePlaybackState.Connecting(session.desktopName))
                updateNotification(session.desktopName, strings.connecting)
            }
        } finally {
            if (activeSession == session && !currentCoroutineContext().isActive) {
                updateNotification(session.desktopName, strings.stopping)
            }
        }
    }

    private suspend fun waitBeforeReconnect() {
        if (isNetworkAvailable()) {
            delay(RECONNECT_DELAY_MS)
            return
        }
        while (currentCoroutineContext().isActive && !isNetworkAvailable()) {
            withTimeoutOrNull(RECONNECT_DELAY_MS) {
                networkSignal.receive()
            }
        }
    }

    private suspend fun playOnce(
        session: PlaybackSession,
        resumeSessionId: String?,
        onSessionConnected: (String) -> Unit,
    ) {
        val socket = DatagramSocket(0)
        var registeredSessionId: String? = null
        var disconnectOnExit = true
        try {
            val response = ControlClient.connect(
                controlUrl = session.controlUrl,
                pin = session.pin,
                udpPort = socket.localPort,
                clientId = session.clientId,
                deviceName = deviceName(session.strings),
                sessionId = resumeSessionId,
            )
            if (!response.ok) {
                throw ControlFailure(response.message.ifBlank { "Pairing was rejected" }, false)
            }
            registeredSessionId = response.sessionId ?: resumeSessionId
            registeredSessionId?.let(onSessionConnected)
            val media = response.media
                ?: throw ControlFailure("Desktop did not provide audio settings", true)
            coroutineScope {
                val heartbeatJob = launch(Dispatchers.IO) {
                    val heartbeatSessionId = registeredSessionId ?: return@launch
                    while (isActive) {
                        delay(HEARTBEAT_INTERVAL_MS)
                        runCatching {
                            ControlClient.heartbeat(
                                session.controlUrl,
                                session.pin,
                                heartbeatSessionId,
                            )
                        }
                    }
                }
                try {
                    RtpAudioReceiver(socket, media.audio) { stats ->
                        updatePlaybackState(
                            LanPulsePlaybackState.Playing(
                                desktopName = session.desktopName,
                                packetsReceived = stats.packetsReceived,
                                packetsLost = stats.packetsLost,
                                bufferMs = stats.bufferMs,
                                queuedMs = stats.queuedMs,
                                jitterMs = stats.jitterMs,
                                audioUnderruns = stats.audioUnderruns,
                                driftInsertedFrames = stats.driftInsertedFrames,
                                driftDroppedFrames = stats.driftDroppedFrames,
                                invalidPackets = stats.invalidPackets,
                                receiveQueueOverflows = stats.receiveQueueOverflows,
                                packetPoolExhausted = stats.packetPoolExhausted,
                                duplicatePackets = stats.duplicatePackets,
                                latePackets = stats.latePackets,
                                replacedPackets = stats.replacedPackets,
                                prunedPackets = stats.prunedPackets,
                            ),
                        )
                        updateNotification(
                            session.desktopName,
                            "${session.strings.playing} - ${stats.bufferMs} ms ${session.strings.buffer}",
                        )
                    }.play()
                } finally {
                    heartbeatJob.cancelAndJoin()
                }
            }
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            disconnectOnExit = false
            throw error
        } finally {
            socket.close()
            if (disconnectOnExit) {
                disconnectRegisteredSession(session, registeredSessionId)
            }
        }
    }

    private fun disconnectRegisteredSession(session: PlaybackSession, sessionId: String?) {
        if (sessionId == null) return
        runCatching {
            ControlClient.disconnect(
                session.controlUrl,
                session.pin,
                sessionId,
            )
        }
    }

    private suspend fun stopSession() = commandMutex.withLock {
        activeSession = null
        playbackJob?.cancelAndJoin()
        playbackJob = null
        updatePlaybackState(LanPulsePlaybackState.Idle)
        releasePlaybackResources()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    private fun failAndStop(message: String) {
        updatePlaybackState(LanPulsePlaybackState.Failed(message))
        releasePlaybackResources()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    private fun startAsForeground(desktopName: String, status: String) {
        val notification = buildNotification(desktopName, status)
        lastNotificationContent = notificationContent(desktopName, status)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(
                NOTIFICATION_ID,
                notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK,
            )
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    private fun updateNotification(desktopName: String, status: String) {
        val content = notificationContent(desktopName, status)
        if (content == lastNotificationContent) return
        lastNotificationContent = content
        getSystemService(NotificationManager::class.java)
            .notify(NOTIFICATION_ID, buildNotification(desktopName, status))
    }

    private fun notificationContent(desktopName: String, status: String): String =
        "$desktopName\u0000$status"

    private fun buildNotification(desktopName: String, status: String): Notification {
        val strings = notificationLanguage.strings()
        val disconnectIntent = PendingIntent.getService(
            this,
            DISCONNECT_REQUEST_CODE,
            Intent(this, LanPulsePlaybackService::class.java).setAction(ACTION_DISCONNECT),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val builder = Notification.Builder(this, NOTIFICATION_CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_sys_headset)
            .setContentTitle(desktopName)
            .setContentText(status)
            .setCategory(Notification.CATEGORY_TRANSPORT)
            .setOnlyAlertOnce(true)
            .setOngoing(true)
            .setShowWhen(false)
            .setStyle(
                Notification.MediaStyle()
                    .setMediaSession(mediaSession?.sessionToken)
                    .setShowActionsInCompactView(0),
            )
            .addAction(
                Notification.Action.Builder(
                    Icon.createWithResource(this, android.R.drawable.ic_media_pause),
                    strings.disconnect,
                    disconnectIntent,
                ).build(),
            )

        packageManager.getLaunchIntentForPackage(packageName)?.let { launchIntent ->
            builder.setContentIntent(
                PendingIntent.getActivity(
                    this,
                    OPEN_APP_REQUEST_CODE,
                    launchIntent,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
                ),
            )
        }
        return builder.build()
    }

    private fun createNotificationChannel() {
        val strings = notificationLanguage.strings()
        val channel = NotificationChannel(
            NOTIFICATION_CHANNEL_ID,
            strings.playbackChannelName,
            NotificationManager.IMPORTANCE_LOW,
        ).apply {
            description = strings.playbackChannelDescription
            setSound(null, null)
            enableVibration(false)
        }
        getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
    }

    private fun createMediaSession(): MediaSession {
        return MediaSession(this, "LanPulse").apply {
            setCallback(
                object : MediaSession.Callback() {
                    override fun onPause() {
                        serviceScope.launch { stopSession() }
                    }

                    override fun onStop() {
                        serviceScope.launch { stopSession() }
                    }
                },
            )
            setFlags(
                MediaSession.FLAG_HANDLES_MEDIA_BUTTONS or
                    MediaSession.FLAG_HANDLES_TRANSPORT_CONTROLS,
            )
            setPlaybackState(mediaPlaybackState(LanPulsePlaybackState.Idle))
        }
    }

    private fun updatePlaybackState(state: LanPulsePlaybackState) {
        AndroidPlaybackSession.update(state)
        mediaSession?.setPlaybackState(mediaPlaybackState(state))
        mediaSession?.isActive = state !is LanPulsePlaybackState.Idle
    }

    private fun mediaPlaybackState(state: LanPulsePlaybackState): MediaPlaybackState =
        MediaPlaybackState.Builder()
            .setState(mediaSessionStateFor(state), 0L, 1f)
            .setActions(MediaPlaybackState.ACTION_STOP or MediaPlaybackState.ACTION_PAUSE)
            .build()

    private fun requestPlaybackFocus(): Boolean {
        val manager = audioManager ?: return false
        val focusRequest = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_MEDIA)
                    .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                    .build(),
            )
            .setOnAudioFocusChangeListener { focusChange ->
                if (shouldStopForAudioFocusChange(focusChange)) {
                    serviceScope.launch { stopSession() }
                }
            }
            .build()
        val granted = manager.requestAudioFocus(focusRequest) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
        if (granted) {
            audioFocusRequest = focusRequest
        }
        return granted
    }

    private fun abandonPlaybackFocus() {
        val request = audioFocusRequest ?: return
        audioFocusRequest = null
        audioManager?.abandonAudioFocusRequest(request)
    }

    private fun acquirePlaybackResources() {
        mediaSession?.isActive = true
        registerOutputDeviceCallback()
        registerNetworkCallback()
        if (wakeLock?.isHeld != true) {
            val powerManager = getSystemService(Context.POWER_SERVICE) as PowerManager
            wakeLock = powerManager.newWakeLock(
                PowerManager.PARTIAL_WAKE_LOCK,
                "$packageName:playback",
            ).apply {
                setReferenceCounted(false)
                acquire()
            }
        }
        if (wifiLock?.isHeld != true) {
            val wifiManager = applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
            @Suppress("DEPRECATION")
            wifiLock = wifiManager.createWifiLock(
                WifiManager.WIFI_MODE_FULL_HIGH_PERF,
                "$packageName:playback",
            ).apply {
                setReferenceCounted(false)
                acquire()
            }
        }
    }

    private fun releasePlaybackResources() {
        wakeLock?.let { if (it.isHeld) it.release() }
        wakeLock = null
        wifiLock?.let { if (it.isHeld) it.release() }
        wifiLock = null
        unregisterOutputDeviceCallback()
        unregisterNetworkCallback()
        abandonPlaybackFocus()
        mediaSession?.isActive = false
        lastNotificationContent = null
    }

    private fun registerOutputDeviceCallback() {
        if (outputDeviceCallback != null) return
        val callback = object : AudioDeviceCallback() {
            override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>) {
                val removedOutputs = removedDevices.count { it.isSink }
                if (shouldStopForOutputDeviceRemoval(removedOutputs)) {
                    serviceScope.launch { stopSession() }
                }
            }
        }
        outputDeviceCallback = callback
        audioManager?.registerAudioDeviceCallback(callback, Handler(Looper.getMainLooper()))
    }

    private fun unregisterOutputDeviceCallback() {
        val callback = outputDeviceCallback ?: return
        outputDeviceCallback = null
        audioManager?.unregisterAudioDeviceCallback(callback)
    }

    private fun registerNetworkCallback() {
        if (networkCallback != null) return
        val manager = connectivityManager ?: return
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                networkSignal.trySend(Unit)
            }

            override fun onLost(network: Network) {
                networkSignal.trySend(Unit)
            }
        }
        networkCallback = callback
        runCatching { manager.registerDefaultNetworkCallback(callback) }
            .onFailure { networkCallback = null }
    }

    private fun unregisterNetworkCallback() {
        val callback = networkCallback ?: return
        networkCallback = null
        runCatching { connectivityManager?.unregisterNetworkCallback(callback) }
    }

    private fun isNetworkAvailable(): Boolean =
        connectivityManager?.activeNetwork != null

    private fun deviceName(strings: MobileStrings): String = listOf(Build.MANUFACTURER, Build.MODEL)
        .filter(String::isNotBlank)
        .distinct()
        .joinToString(" ")
        .ifBlank { strings.androidDevice }

    private fun Intent.toPlaybackSession(): PlaybackSession? {
        val desktopName = getStringExtra(EXTRA_DESKTOP_NAME)?.takeIf(String::isNotBlank)
            ?: return null
        val controlUrl = getStringExtra(EXTRA_CONTROL_URL)?.takeIf(String::isNotBlank)
            ?: return null
        val pin = getStringExtra(EXTRA_PIN)?.takeIf(String::isNotBlank) ?: return null
        val clientId = getStringExtra(EXTRA_CLIENT_ID)?.takeIf(String::isNotBlank) ?: return null
        val language = MobileLanguage.fromCode(getStringExtra(EXTRA_LANGUAGE))
        return PlaybackSession(desktopName, controlUrl, pin, clientId, language)
    }

    private data class PlaybackSession(
        val desktopName: String,
        val controlUrl: String,
        val pin: String,
        val clientId: String,
        val language: MobileLanguage,
    ) {
        val strings: MobileStrings = language.strings()
    }

    companion object {
        const val ACTION_CONNECT = "com.lanpulse.mobile.action.CONNECT"
        const val ACTION_DISCONNECT = "com.lanpulse.mobile.action.DISCONNECT"
        const val EXTRA_DESKTOP_NAME = "desktop_name"
        const val EXTRA_CONTROL_URL = "control_url"
        const val EXTRA_PIN = "pin"
        const val EXTRA_CLIENT_ID = "client_id"
        const val EXTRA_LANGUAGE = "language"

        private const val HEARTBEAT_INTERVAL_MS = 5_000L

        private const val NOTIFICATION_CHANNEL_ID = "lanpulse_playback"
        private const val NOTIFICATION_ID = 4100
        private const val DISCONNECT_REQUEST_CODE = 4101
        private const val OPEN_APP_REQUEST_CODE = 4102
        private const val RECONNECT_DELAY_MS = 3_000L
    }
}
