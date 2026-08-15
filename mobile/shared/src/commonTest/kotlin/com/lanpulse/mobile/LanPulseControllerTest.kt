package com.lanpulse.mobile

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class LanPulseControllerTest {
    @Test
    fun playbackModeSelectionIsPersistedWhileIdle() = withController { controller, client ->
        assertTrue(controller.state.value.supportsPlaybackMode)
        assertEquals(PlaybackMode.Adaptive, controller.state.value.playbackMode)

        controller.selectPlaybackMode(PlaybackMode.Immediate)

        assertEquals(PlaybackMode.Immediate, controller.state.value.playbackMode)
        assertEquals(listOf(PlaybackMode.Immediate), client.savedPlaybackModes)
    }

    @Test
    fun playbackModeSelectionIsIgnoredDuringPlayback() = withController { controller, client ->
        client.mutablePlaybackState.value = PlaybackState.Playing(
            desktopName = "Desktop",
            packetsReceived = 1,
            packetsLost = 0,
            bufferMs = 120,
        )

        controller.selectPlaybackMode(PlaybackMode.Immediate)

        assertEquals(PlaybackMode.Adaptive, controller.state.value.playbackMode)
        assertTrue(client.savedPlaybackModes.isEmpty())
    }

    @Test
    fun playbackModeSelectionIsIgnoredOnUnsupportedPlatforms() {
        val client = FakeLanPulseClient(supportsPlaybackMode = false)

        withController(client) { controller, fakeClient ->
            assertFalse(controller.state.value.supportsPlaybackMode)

            controller.selectPlaybackMode(PlaybackMode.Immediate)

            assertEquals(PlaybackMode.Adaptive, controller.state.value.playbackMode)
            assertTrue(fakeClient.savedPlaybackModes.isEmpty())
        }
    }

    private fun withController(
        client: FakeLanPulseClient = FakeLanPulseClient(),
        block: (LanPulseController, FakeLanPulseClient) -> Unit,
    ) {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        try {
            block(LanPulseController(client, scope), client)
        } finally {
            scope.cancel()
        }
    }
}

private class FakeLanPulseClient(
    override val supportsPlaybackMode: Boolean = true,
) : LanPulseClient {
    val mutablePlaybackState = MutableStateFlow<PlaybackState>(PlaybackState.Idle)
    val savedPlaybackModes = mutableListOf<PlaybackMode>()

    override val playbackState: StateFlow<PlaybackState> = mutablePlaybackState

    override suspend fun discover(): List<DesktopEndpoint> = emptyList()

    override fun connect(
        endpoint: DesktopEndpoint,
        pin: String,
        language: MobileLanguage,
        playbackMode: PlaybackMode,
    ) = Unit

    override fun disconnect() = Unit

    override fun savePlaybackMode(playbackMode: PlaybackMode) {
        savedPlaybackModes += playbackMode
    }
}
