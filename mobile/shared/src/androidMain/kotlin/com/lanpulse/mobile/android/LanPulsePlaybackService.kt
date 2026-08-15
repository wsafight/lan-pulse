package com.lanpulse.mobile.android

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
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
import android.net.NetworkCapabilities
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.IBinder
import android.os.PowerManager
import android.os.Process
import android.os.SystemClock
import com.lanpulse.mobile.MobileLanguage
import com.lanpulse.mobile.MobileStrings
import com.lanpulse.mobile.LANPULSE_CAPABILITY_RTP_NACK_V1
import com.lanpulse.mobile.PlaybackMode
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
                Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
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
    private var screenReceiver: BroadcastReceiver? = null
    private var mediaSession: MediaSession? = null
    private var lastNotificationContent: String? = null
    private var lastPlaybackStateName: String? = null
    private val networkSignal = Channel<Unit>(Channel.CONFLATED)

    override fun onCreate() {
        super.onCreate()
        AndroidDiagnosticLog.initialize(applicationContext)
        AndroidDiagnosticLog.event("service_created", powerState())
        notificationLanguage = AndroidLanguageStore.load(this)
        audioManager = getSystemService(Context.AUDIO_SERVICE) as AudioManager
        connectivityManager = getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        mediaSession = createMediaSession()
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        AndroidDiagnosticLog.event(
            "service_start_command",
            "action=${intent?.action ?: "null"} flags=$flags start_id=$startId ${powerState()}",
        )
        when (intent?.action) {
            ACTION_CONNECT -> {
                val session = intent.toPlaybackSession()
                if (session == null) {
                    AndroidDiagnosticLog.event("connect_intent_invalid")
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
            else -> {
                AndroidDiagnosticLog.event("service_unknown_command_stopping")
                stopSelf(startId)
            }
        }
        return START_REDELIVER_INTENT
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        AndroidDiagnosticLog.event("service_destroying", powerState())
        playbackJob?.cancel()
        releasePlaybackResources()
        mediaSession?.release()
        mediaSession = null
        networkSignal.close()
        serviceScope.cancel()
        audioDispatcher.close()
        super.onDestroy()
    }

    override fun onTaskRemoved(rootIntent: Intent?) {
        AndroidDiagnosticLog.event("task_removed", powerState())
        super.onTaskRemoved(rootIntent)
    }

    override fun onTrimMemory(level: Int) {
        AndroidDiagnosticLog.event("trim_memory", "level=$level ${powerState()}")
        super.onTrimMemory(level)
    }

    private suspend fun replaceSession(session: PlaybackSession) = commandMutex.withLock {
        AndroidDiagnosticLog.event(
            "session_replacing",
            "desktop=${session.desktopName} control_url=${session.controlUrl} " +
                "playback_mode=${session.playbackMode.storageValue}",
        )
        playbackJob?.cancelAndJoin()

        activeSession = session
        if (!requestPlaybackFocus()) {
            AndroidDiagnosticLog.event("audio_focus_request_denied")
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
        var attempt = 0
        try {
            while (currentCoroutineContext().isActive) {
                attempt += 1
                AndroidDiagnosticLog.event(
                    "playback_attempt_started",
                    "attempt=$attempt resume_session=${reconnectSessionId ?: "none"} ${powerState()}",
                )
                try {
                    playOnce(session, reconnectSessionId) { sessionId ->
                        reconnectSessionId = sessionId
                    }
                    lastFailure = strings.audioStreamEnded
                } catch (error: CancellationException) {
                    AndroidDiagnosticLog.event("playback_attempt_cancelled", "attempt=$attempt")
                    throw error
                } catch (error: ControlFailure) {
                    AndroidDiagnosticLog.error(
                        "control_failure",
                        error,
                        "attempt=$attempt retryable=${error.retryable}",
                    )
                    lastFailure = playbackFailureMessage(error.message, strings)
                    if (!error.retryable) {
                        disconnectRegisteredSession(session, reconnectSessionId)
                        reconnectSessionId = null
                        failAndStop(lastFailure)
                        return
                    }
                } catch (error: InvalidAudioConfigException) {
                    AndroidDiagnosticLog.error("invalid_audio_config", error, "attempt=$attempt")
                    disconnectRegisteredSession(session, reconnectSessionId)
                    reconnectSessionId = null
                    failAndStop(strings.incompatibleAudioFormat)
                    return
                } catch (error: Exception) {
                    AndroidDiagnosticLog.error("playback_attempt_failed", error, "attempt=$attempt")
                    lastFailure = strings.audioPlaybackStopped
                }

                updatePlaybackState(
                    LanPulsePlaybackState.Reconnecting(session.desktopName, lastFailure),
                )
                updateNotification(
                    session.desktopName,
                    reconnectStatusMessage(isNetworkAvailable(), strings),
                )
                AndroidDiagnosticLog.event(
                    "reconnect_waiting",
                    "attempt=$attempt network_available=${isNetworkAvailable()}",
                )
                waitBeforeReconnect()
                updatePlaybackState(LanPulsePlaybackState.Connecting(session.desktopName))
                updateNotification(session.desktopName, strings.connecting)
            }
        } finally {
            AndroidDiagnosticLog.event(
                "playback_loop_finished",
                "active_session_matches=${activeSession == session}",
            )
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
            AndroidDiagnosticLog.event("reconnect_waiting_for_network", powerState())
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
            AndroidDiagnosticLog.event(
                "control_connect_start",
                "url=${session.controlUrl} udp_port=${socket.localPort} " +
                    "resume_session=${resumeSessionId ?: "none"}",
            )
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
            AndroidDiagnosticLog.event(
                "control_connected",
                "session=${registeredSessionId ?: "none"} target=${media.targetIp}:${media.targetPort} " +
                    "sample_rate=${media.audio.sampleRate} channels=${media.audio.channels} " +
                    "packet_ms=${media.audio.packetMs} ssrc=${media.audio.ssrc}",
            )
            coroutineScope {
                val heartbeatJob = launch(Dispatchers.IO) {
                    val heartbeatSessionId = registeredSessionId ?: return@launch
                    var successfulHeartbeats = 0
                    while (isActive) {
                        runCatching {
                            ControlClient.heartbeat(
                                session.controlUrl,
                                session.pin,
                                heartbeatSessionId,
                            )
                        }.onSuccess {
                            successfulHeartbeats += 1
                            if (successfulHeartbeats == 1 || successfulHeartbeats % 10 == 0) {
                                AndroidDiagnosticLog.event(
                                    "heartbeat_ok",
                                    "session=$heartbeatSessionId count=$successfulHeartbeats",
                                )
                            }
                        }.onFailure { error ->
                            AndroidDiagnosticLog.error(
                                "heartbeat_failed",
                                error,
                                "session=$heartbeatSessionId",
                            )
                        }
                        delay(HEARTBEAT_INTERVAL_MS)
                    }
                }
                try {
                    var lastStatsLogMs = 0L
                    RtpAudioReceiver(
                        socket = socket,
                        audio = media.audio,
                        playbackMode = session.playbackMode,
                        onStats = { stats ->
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
                            val now = SystemClock.elapsedRealtime()
                            if (now - lastStatsLogMs >= DIAGNOSTIC_STATS_INTERVAL_MS) {
                                lastStatsLogMs = now
                                AndroidDiagnosticLog.event("rtp_stats", stats.diagnosticSummary())
                            }
                        },
                        onDiagnostic = { message ->
                            AndroidDiagnosticLog.event("rtp_event", message)
                        },
                        enableNack = session.playbackMode == PlaybackMode.Adaptive &&
                            LANPULSE_CAPABILITY_RTP_NACK_V1 in response.capabilities,
                    ).play()
                } finally {
                    heartbeatJob.cancelAndJoin()
                }
            }
        } catch (error: CancellationException) {
            throw error
        } catch (error: Exception) {
            AndroidDiagnosticLog.error(
                "play_once_failed",
                error,
                "session=${registeredSessionId ?: "none"}",
            )
            disconnectOnExit = false
            throw error
        } finally {
            AndroidDiagnosticLog.event(
                "play_once_finished",
                "session=${registeredSessionId ?: "none"} notify_desktop=$disconnectOnExit",
            )
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
        }.onSuccess {
            AndroidDiagnosticLog.event("control_disconnected", "session=$sessionId")
        }.onFailure { error ->
            AndroidDiagnosticLog.error(
                "control_disconnect_failed",
                error,
                "session=$sessionId",
            )
        }
    }

    private suspend fun stopSession() = commandMutex.withLock {
        AndroidDiagnosticLog.event("session_stopping", powerState())
        activeSession = null
        playbackJob?.cancelAndJoin()
        playbackJob = null
        updatePlaybackState(LanPulsePlaybackState.Idle)
        releasePlaybackResources()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    private fun failAndStop(message: String) {
        AndroidDiagnosticLog.event("session_failed_stopping", "message=$message ${powerState()}")
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
        val stateName = state.javaClass.simpleName
        if (lastPlaybackStateName != stateName) {
            AndroidDiagnosticLog.event("playback_state", "state=$stateName value=$state")
            lastPlaybackStateName = stateName
        }
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
                val shouldStop = shouldStopForAudioFocusChange(focusChange)
                AndroidDiagnosticLog.event(
                    "audio_focus_changed",
                    "change=${audioFocusName(focusChange)} stop=$shouldStop ${powerState()}",
                )
                if (shouldStop) {
                    serviceScope.launch { stopSession() }
                }
            }
            .build()
        val granted = manager.requestAudioFocus(focusRequest) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
        AndroidDiagnosticLog.event("audio_focus_requested", "granted=$granted")
        if (granted) {
            audioFocusRequest = focusRequest
        }
        return granted
    }

    private fun abandonPlaybackFocus() {
        val request = audioFocusRequest ?: return
        audioFocusRequest = null
        audioManager?.abandonAudioFocusRequest(request)
        AndroidDiagnosticLog.event("audio_focus_abandoned")
    }

    private fun acquirePlaybackResources() {
        mediaSession?.isActive = true
        registerOutputDeviceCallback()
        registerNetworkCallback()
        registerScreenReceiver()
        if (wakeLock?.isHeld != true) {
            val powerManager = getSystemService(Context.POWER_SERVICE) as PowerManager
            wakeLock = powerManager.newWakeLock(
                PowerManager.PARTIAL_WAKE_LOCK,
                "$packageName:playback",
            ).apply {
                setReferenceCounted(false)
                acquire()
            }
            AndroidDiagnosticLog.event("wake_lock_acquired", powerState())
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
            AndroidDiagnosticLog.event("wifi_lock_acquired", powerState())
        }
        AndroidDiagnosticLog.event("playback_resources_acquired", resourceState())
    }

    private fun releasePlaybackResources() {
        AndroidDiagnosticLog.event("playback_resources_releasing", resourceState())
        wakeLock?.let { if (it.isHeld) it.release() }
        wakeLock = null
        wifiLock?.let { if (it.isHeld) it.release() }
        wifiLock = null
        unregisterOutputDeviceCallback()
        unregisterNetworkCallback()
        unregisterScreenReceiver()
        abandonPlaybackFocus()
        mediaSession?.isActive = false
        lastNotificationContent = null
        AndroidDiagnosticLog.event("playback_resources_released", resourceState())
    }

    private fun registerOutputDeviceCallback() {
        if (outputDeviceCallback != null) return
        val callback = object : AudioDeviceCallback() {
            override fun onAudioDevicesAdded(addedDevices: Array<out AudioDeviceInfo>) {
                AndroidDiagnosticLog.event(
                    "audio_devices_added",
                    "devices=${audioDevicesDescription(addedDevices)}",
                )
            }

            override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>) {
                val privateOutputRemoved = removedDevices.any {
                    it.isSink && shouldStopForOutputDeviceRemoval(it.type)
                }
                AndroidDiagnosticLog.event(
                    "audio_devices_removed",
                    "devices=${audioDevicesDescription(removedDevices)} stop=$privateOutputRemoved",
                )
                if (privateOutputRemoved) {
                    serviceScope.launch { stopSession() }
                }
            }
        }
        outputDeviceCallback = callback
        audioManager?.registerAudioDeviceCallback(callback, Handler(Looper.getMainLooper()))
        val outputs = audioManager?.getDevices(AudioManager.GET_DEVICES_OUTPUTS).orEmpty()
        AndroidDiagnosticLog.event(
            "audio_device_callback_registered",
            "outputs=${audioDevicesDescription(outputs)}",
        )
    }

    private fun unregisterOutputDeviceCallback() {
        val callback = outputDeviceCallback ?: return
        outputDeviceCallback = null
        audioManager?.unregisterAudioDeviceCallback(callback)
        AndroidDiagnosticLog.event("audio_device_callback_unregistered")
    }

    private fun registerNetworkCallback() {
        if (networkCallback != null) return
        val manager = connectivityManager ?: return
        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) {
                AndroidDiagnosticLog.event(
                    "network_available",
                    networkDescription(network),
                )
                networkSignal.trySend(Unit)
            }

            override fun onLost(network: Network) {
                AndroidDiagnosticLog.event("network_lost", "network=$network ${powerState()}")
                networkSignal.trySend(Unit)
            }

            override fun onCapabilitiesChanged(
                network: Network,
                networkCapabilities: NetworkCapabilities,
            ) {
                AndroidDiagnosticLog.event(
                    "network_capabilities_changed",
                    "network=$network ${capabilitiesDescription(networkCapabilities)}",
                )
                networkSignal.trySend(Unit)
            }

            override fun onBlockedStatusChanged(network: Network, blocked: Boolean) {
                AndroidDiagnosticLog.event(
                    "network_blocked_changed",
                    "network=$network blocked=$blocked ${powerState()}",
                )
            }
        }
        networkCallback = callback
        runCatching { manager.registerDefaultNetworkCallback(callback) }
            .onSuccess {
                AndroidDiagnosticLog.event(
                    "network_callback_registered",
                    activeNetworkDescription(),
                )
            }
            .onFailure { error ->
                networkCallback = null
                AndroidDiagnosticLog.error("network_callback_register_failed", error)
            }
    }

    private fun unregisterNetworkCallback() {
        val callback = networkCallback ?: return
        networkCallback = null
        runCatching { connectivityManager?.unregisterNetworkCallback(callback) }
            .onSuccess { AndroidDiagnosticLog.event("network_callback_unregistered") }
            .onFailure { AndroidDiagnosticLog.error("network_callback_unregister_failed", it) }
    }

    private fun registerScreenReceiver() {
        if (screenReceiver != null) return
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context?, intent: Intent?) {
                AndroidDiagnosticLog.event(
                    "screen_state_changed",
                    "action=${intent?.action ?: "null"} ${powerState()} ${activeNetworkDescription()}",
                )
            }
        }
        val filter = IntentFilter().apply {
            addAction(Intent.ACTION_SCREEN_OFF)
            addAction(Intent.ACTION_SCREEN_ON)
            addAction(Intent.ACTION_USER_PRESENT)
        }
        screenReceiver = receiver
        runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED)
            } else {
                @Suppress("DEPRECATION")
                registerReceiver(receiver, filter)
            }
        }.onSuccess {
            AndroidDiagnosticLog.event("screen_receiver_registered", powerState())
        }.onFailure { error ->
            screenReceiver = null
            AndroidDiagnosticLog.error("screen_receiver_register_failed", error)
        }
    }

    private fun unregisterScreenReceiver() {
        val receiver = screenReceiver ?: return
        screenReceiver = null
        runCatching { unregisterReceiver(receiver) }
            .onSuccess { AndroidDiagnosticLog.event("screen_receiver_unregistered") }
            .onFailure { AndroidDiagnosticLog.error("screen_receiver_unregister_failed", it) }
    }

    private fun isNetworkAvailable(): Boolean =
        connectivityManager?.activeNetwork != null

    private fun powerState(): String {
        val manager = getSystemService(Context.POWER_SERVICE) as PowerManager
        return "interactive=${manager.isInteractive} idle=${manager.isDeviceIdleMode} " +
            "power_save=${manager.isPowerSaveMode}"
    }

    private fun resourceState(): String =
        "wake_lock=${wakeLock?.isHeld == true} wifi_lock=${wifiLock?.isHeld == true} " +
            "media_session=${mediaSession?.isActive == true} ${powerState()}"

    private fun activeNetworkDescription(): String {
        val manager = connectivityManager ?: return "active_network=manager_unavailable"
        val network = manager.activeNetwork ?: return "active_network=none"
        return "active_${networkDescription(network)}"
    }

    private fun networkDescription(network: Network): String {
        val capabilities = connectivityManager?.getNetworkCapabilities(network)
        return "network=$network ${capabilitiesDescription(capabilities)} ${powerState()}"
    }

    private fun capabilitiesDescription(capabilities: NetworkCapabilities?): String {
        if (capabilities == null) return "capabilities=none"
        val transports = buildList {
            if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_WIFI)) add("wifi")
            if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_CELLULAR)) add("cellular")
            if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)) add("ethernet")
            if (capabilities.hasTransport(NetworkCapabilities.TRANSPORT_VPN)) add("vpn")
        }.ifEmpty { listOf("other") }.joinToString(",")
        return "transports=$transports " +
            "internet=${capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)} " +
            "validated=${capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_VALIDATED)} " +
            "not_suspended=${capabilities.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_SUSPENDED)}"
    }

    private fun audioDevicesDescription(devices: Array<out AudioDeviceInfo>): String = devices
        .joinToString(",") { device ->
            "${device.type}:${device.productName}:sink=${device.isSink}"
        }
        .ifBlank { "none" }

    private fun audioFocusName(change: Int): String = when (change) {
        AudioManager.AUDIOFOCUS_GAIN -> "gain"
        AudioManager.AUDIOFOCUS_LOSS -> "loss"
        AudioManager.AUDIOFOCUS_LOSS_TRANSIENT -> "loss_transient"
        AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK -> "loss_transient_duck"
        else -> change.toString()
    }

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
        val playbackMode = PlaybackMode.fromStorageValue(getStringExtra(EXTRA_PLAYBACK_MODE))
        return PlaybackSession(desktopName, controlUrl, pin, clientId, language, playbackMode)
    }

    private data class PlaybackSession(
        val desktopName: String,
        val controlUrl: String,
        val pin: String,
        val clientId: String,
        val language: MobileLanguage,
        val playbackMode: PlaybackMode,
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
        const val EXTRA_PLAYBACK_MODE = "playback_mode"

        private const val HEARTBEAT_INTERVAL_MS = 3_000L
        private const val DIAGNOSTIC_STATS_INTERVAL_MS = 2_000L

        private const val NOTIFICATION_CHANNEL_ID = "lanpulse_playback"
        private const val NOTIFICATION_ID = 4100
        private const val DISCONNECT_REQUEST_CODE = 4101
        private const val OPEN_APP_REQUEST_CODE = 4102
        private const val RECONNECT_DELAY_MS = 3_000L
    }
}

private fun ReceiverStats.diagnosticSummary(): String =
        "received=$packetsReceived lost=$packetsLost buffer_ms=$bufferMs queued_ms=$queuedMs " +
        "software_queued_ms=$softwareQueuedMs output_queued_ms=$outputQueuedMs " +
        "jitter_ms=${"%.2f".format(java.util.Locale.US, jitterMs)} underruns=$audioUnderruns " +
        "drift_inserted=$driftInsertedFrames drift_dropped=$driftDroppedFrames " +
        "invalid=$invalidPackets queue_overflows=$receiveQueueOverflows " +
        "pool_exhausted=$packetPoolExhausted duplicates=$duplicatePackets late=$latePackets " +
        "replaced=$replacedPackets pruned=$prunedPackets " +
        "max_receive_gap_ms=$maxReceiveGapMs max_dispatch_delay_ms=$maxDispatchDelayMs " +
        "max_audio_write_ms=$maxAudioWriteMs output_dropped_bytes=$outputDroppedBytes " +
        "nack_requests=$nackRequests " +
        "nack_recoveries=$nackRecoveries"
