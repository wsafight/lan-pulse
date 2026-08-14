package com.lanpulse.mobile.android

import com.lanpulse.mobile.DesktopEndpoint
import com.lanpulse.mobile.DiscoveryResponse
import com.lanpulse.mobile.lanPulseJson
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.NetworkInterface
import java.net.SocketTimeoutException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

internal object UdpDiscoveryClient {
    private const val MAGIC = "LANPULSE_DISCOVER_V1"
    private const val FIRST_PORT = 41000
    private const val LAST_PORT = 41020
    private const val TIMEOUT_MS = 1_200L

    suspend fun discover(): List<DesktopEndpoint> = withContext(Dispatchers.IO) {
        DatagramSocket(null).use { socket ->
            socket.reuseAddress = true
            socket.broadcast = true
            socket.soTimeout = 120
            socket.bind(InetSocketAddress(0))

            val probe = MAGIC.encodeToByteArray()
            val targets = broadcastAddresses()
            for (address in targets) {
                for (port in FIRST_PORT..LAST_PORT) {
                    runCatching {
                        socket.send(DatagramPacket(probe, probe.size, address, port))
                    }
                }
            }

            val responses = linkedMapOf<String, DesktopEndpoint>()
            val deadline = System.nanoTime() + TIMEOUT_MS * 1_000_000L
            val buffer = ByteArray(4_096)
            val packet = DatagramPacket(buffer, buffer.size)
            while (System.nanoTime() < deadline) {
                packet.length = buffer.size
                try {
                    socket.receive(packet)
                    val payload = packet.data.decodeToString(packet.offset, packet.offset + packet.length)
                    val response = runCatching {
                        lanPulseJson.decodeFromString<DiscoveryResponse>(payload)
                    }.getOrNull() ?: continue
                    if (response.type != "lanpulse.desktop.v1") continue
                    if (!response.isProtocolCompatible()) continue
                    val endpoint = DesktopEndpoint(
                        name = response.name,
                        controlUrl = response.controlUrl.trimEnd('/'),
                        audio = response.audio,
                        protocolVersion = response.protocolVersion,
                        minSupportedProtocolVersion = response.minSupportedProtocolVersion,
                        capabilities = response.capabilities,
                    )
                    responses[endpoint.id] = endpoint
                } catch (_: SocketTimeoutException) {
                    // Keep receiving until the overall discovery deadline.
                }
            }
            responses.values.sortedBy { it.name.lowercase() }
        }
    }

    private fun broadcastAddresses(): Set<InetAddress> {
        val addresses = linkedSetOf(InetAddress.getByName("255.255.255.255"))
        val interfaces = runCatching { NetworkInterface.getNetworkInterfaces() }.getOrNull()
            ?: return addresses
        while (interfaces.hasMoreElements()) {
            val networkInterface = interfaces.nextElement()
            if (!runCatching { networkInterface.isUp }.getOrDefault(false)) continue
            if (runCatching { networkInterface.isLoopback }.getOrDefault(true)) continue
            networkInterface.interfaceAddresses.mapNotNullTo(addresses) { it.broadcast }
        }
        return addresses
    }
}
