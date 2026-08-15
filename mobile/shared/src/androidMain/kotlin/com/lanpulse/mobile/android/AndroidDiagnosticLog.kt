package com.lanpulse.mobile.android

import android.content.Context
import android.os.Build
import android.os.Process
import android.os.SystemClock
import android.util.Log
import java.io.File
import java.io.FileOutputStream
import java.time.OffsetDateTime
import java.time.format.DateTimeFormatter
import java.util.concurrent.Executors

internal object AndroidDiagnosticLog {
    private const val TAG = "LanPulse"
    private const val DIRECTORY = "diagnostics"
    private const val CURRENT_LOG = "lanpulse-mobile.log"
    private const val PREVIOUS_LOG = "lanpulse-mobile.previous.log"
    private const val MAX_LOG_BYTES = 4L * 1024L * 1024L

    private val writer = Executors.newSingleThreadExecutor { command ->
        Thread(command, "LanPulseDiagnostic").apply {
            priority = Thread.NORM_PRIORITY - 1
        }
    }

    @Volatile
    private var logDirectory: File? = null

    fun initialize(context: Context) {
        if (logDirectory != null) return
        synchronized(this) {
            if (logDirectory != null) return
            logDirectory = File(context.filesDir, DIRECTORY).also { it.mkdirs() }
            val packageInfo = runCatching {
                context.packageManager.getPackageInfo(context.packageName, 0)
            }.getOrNull()
            event(
                "diagnostics_initialized",
                "version=${packageInfo?.versionName ?: "unknown"} " +
                    "sdk=${Build.VERSION.SDK_INT} device=${Build.MANUFACTURER}/${Build.MODEL}",
            )
        }
    }

    fun event(name: String, details: String = "") {
        val cleanDetails = details
            .replace('\n', ' ')
            .replace('\r', ' ')
            .take(MAX_DETAILS_LENGTH)
        val message = if (cleanDetails.isBlank()) name else "$name $cleanDetails"
        Log.i(TAG, message)

        val directory = logDirectory ?: return
        val line = buildString {
            append(OffsetDateTime.now().format(DateTimeFormatter.ISO_OFFSET_DATE_TIME))
            append(" elapsed_ms=")
            append(SystemClock.elapsedRealtime())
            append(" pid=")
            append(Process.myPid())
            append(" thread=")
            append(Thread.currentThread().name)
            append(" event=")
            append(message)
            append('\n')
        }
        writer.execute { append(directory, line) }
    }

    fun error(name: String, error: Throwable, details: String = "") {
        val cause = "${error.javaClass.simpleName}:${error.message.orEmpty()}"
        event(name, listOf(details, "error=$cause").filter(String::isNotBlank).joinToString(" "))
    }

    private fun append(directory: File, line: String) {
        runCatching {
            directory.mkdirs()
            val current = File(directory, CURRENT_LOG)
            if (current.length() >= MAX_LOG_BYTES) {
                val previous = File(directory, PREVIOUS_LOG)
                previous.delete()
                current.renameTo(previous)
            }
            FileOutputStream(current, true).bufferedWriter().use { it.write(line) }
        }.onFailure { error ->
            Log.e(TAG, "diagnostic_log_write_failed", error)
        }
    }

    private const val MAX_DETAILS_LENGTH = 8_000
}
