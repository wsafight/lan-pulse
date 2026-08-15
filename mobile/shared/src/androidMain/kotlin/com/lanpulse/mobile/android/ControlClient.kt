package com.lanpulse.mobile.android

import com.lanpulse.mobile.ConnectRequest
import com.lanpulse.mobile.ConnectResponse
import com.lanpulse.mobile.DisconnectRequest
import com.lanpulse.mobile.HeartbeatRequest
import com.lanpulse.mobile.lanPulseJson
import java.net.HttpURLConnection
import java.net.URL
import kotlinx.serialization.encodeToString

internal const val CONTROL_ERROR_INVALID_PIN = "Invalid PIN"
internal const val CONTROL_ERROR_PIN_EXPIRED = "PIN expired"
internal const val CONTROL_ERROR_DEVICE_BUSY = "Another device is already connected"
internal const val CONTROL_ERROR_PROTOCOL_INCOMPATIBLE = "Protocol version is not compatible"
internal const val CONTROL_ERROR_TOO_MANY_PAIRING_ATTEMPTS = "Too many failed pairing attempts"

private const val HTTP_UPGRADE_REQUIRED = 426
private const val HTTP_TOO_MANY_REQUESTS = 429

internal object ControlClient {
    private const val CONNECT_TIMEOUT_MS = 2_000
    private const val READ_TIMEOUT_MS = 3_000

    fun connect(
        controlUrl: String,
        pin: String,
        udpPort: Int,
        clientId: String,
        deviceName: String,
        sessionId: String? = null,
    ): ConnectResponse {
        val request = ConnectRequest(pin, udpPort, clientId, deviceName, sessionId)
        val body = post(controlUrl, "/api/connect", lanPulseJson.encodeToString(request))
        val response = runCatching { lanPulseJson.decodeFromString<ConnectResponse>(body) }
            .getOrElse { throw ControlFailure("Invalid response from desktop", retryable = true) }
        if (!response.isProtocolCompatible()) {
            throw ControlFailure(CONTROL_ERROR_PROTOCOL_INCOMPATIBLE, retryable = false)
        }
        return response
    }

    fun disconnect(controlUrl: String, pin: String, sessionId: String? = null) {
        post(
            controlUrl,
            "/api/disconnect",
            lanPulseJson.encodeToString(DisconnectRequest(pin, sessionId)),
        )
    }

    fun heartbeat(controlUrl: String, pin: String, sessionId: String) {
        post(
            controlUrl,
            "/api/heartbeat",
            lanPulseJson.encodeToString(HeartbeatRequest(pin, sessionId)),
        )
    }

    private fun post(controlUrl: String, path: String, payload: String): String {
        val connection = runCatching {
            URL(controlUrl.trimEnd('/') + path).openConnection() as HttpURLConnection
        }.getOrElse { throw ControlFailure("Invalid desktop address", retryable = false) }

        return try {
            connection.requestMethod = "POST"
            connection.connectTimeout = CONNECT_TIMEOUT_MS
            connection.readTimeout = READ_TIMEOUT_MS
            connection.doOutput = true
            connection.setRequestProperty("Content-Type", "application/json")
            connection.setRequestProperty("Accept", "application/json")
            val bytes = payload.encodeToByteArray()
            connection.setFixedLengthStreamingMode(bytes.size)
            connection.outputStream.use { it.write(bytes) }

            val status = connection.responseCode
            val stream = if (status in 200..299) connection.inputStream else connection.errorStream
            val response = stream?.bufferedReader()?.use { it.readText() }.orEmpty()
            if (status !in 200..299) {
                throw ControlFailure(controlErrorMessage(status, response), retryable = status >= 500)
            }
            response
        } catch (error: ControlFailure) {
            throw error
        } catch (error: Exception) {
            throw ControlFailure(error.message ?: "Desktop is unreachable", retryable = true)
        } finally {
            connection.disconnect()
        }
    }
}

internal fun controlErrorMessage(status: Int, response: String): String {
    val desktopMessage = runCatching {
        lanPulseJson.decodeFromString<ConnectResponse>(response).message
    }.getOrNull()

    return when (status) {
        HttpURLConnection.HTTP_UNAUTHORIZED -> when (desktopMessage) {
            "pin expired" -> CONTROL_ERROR_PIN_EXPIRED
            else -> CONTROL_ERROR_INVALID_PIN
        }
        HttpURLConnection.HTTP_CONFLICT -> CONTROL_ERROR_DEVICE_BUSY
        HTTP_UPGRADE_REQUIRED -> CONTROL_ERROR_PROTOCOL_INCOMPATIBLE
        HTTP_TOO_MANY_REQUESTS -> CONTROL_ERROR_TOO_MANY_PAIRING_ATTEMPTS
        else -> response.ifBlank { "Desktop returned HTTP $status" }
    }
}

internal class ControlFailure(
    message: String,
    val retryable: Boolean,
) : Exception(message)
