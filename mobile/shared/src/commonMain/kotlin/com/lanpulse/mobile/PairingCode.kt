package com.lanpulse.mobile

data class PairingCode(
    val controlUrl: String,
    val pin: String,
)

fun parsePairingCode(value: String): PairingCode? {
    val trimmed = value.trim()
    if (!trimmed.startsWith(PAIRING_PREFIX) || '#' in trimmed) return null

    val parameters = mutableMapOf<String, String>()
    val query = trimmed.removePrefix(PAIRING_PREFIX)
    for (entry in query.split('&')) {
        val separator = entry.indexOf('=')
        if (separator <= 0) return null
        val key = decodeQueryComponent(entry.substring(0, separator)) ?: return null
        val parameterValue = decodeQueryComponent(entry.substring(separator + 1)) ?: return null
        if (parameters.put(key, parameterValue) != null) return null
    }

    val pin = parameters["pin"]?.takeIf(::isValidPin) ?: return null
    val endpoint = parameters["url"]?.let(::manualEndpoint) ?: return null
    if (!isLocalIpv4Url(endpoint.controlUrl)) return null
    return PairingCode(endpoint.controlUrl, pin)
}

private fun decodeQueryComponent(value: String): String? = buildString(value.length) {
    var index = 0
    while (index < value.length) {
        when (val char = value[index]) {
            '%' -> {
                if (index + 2 >= value.length) return null
                val high = value[index + 1].digitToIntOrNull(16) ?: return null
                val low = value[index + 2].digitToIntOrNull(16) ?: return null
                val decoded = (high shl 4) or low
                if (decoded !in 0x20..0x7E) return null
                append(decoded.toChar())
                index += 3
            }

            '+' -> {
                append(' ')
                index += 1
            }

            else -> {
                if (char.code !in 0x20..0x7E) return null
                append(char)
                index += 1
            }
        }
    }
}

private fun isLocalIpv4Url(url: String): Boolean {
    val host = url.removePrefix("http://").substringBeforeLast(':')
    val octets = host.split('.').map { it.toIntOrNull() ?: return false }
    if (octets.size != 4 || octets.any { it !in 0..255 }) return false
    return when {
        octets[0] == 10 -> true
        octets[0] == 127 -> true
        octets[0] == 169 && octets[1] == 254 -> true
        octets[0] == 172 && octets[1] in 16..31 -> true
        octets[0] == 192 && octets[1] == 168 -> true
        else -> false
    }
}

private const val PAIRING_PREFIX = "lanpulse://pair?"
