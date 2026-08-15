package com.lanpulse.mobile

import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.Button
import androidx.compose.material.ButtonDefaults
import androidx.compose.material.Card
import androidx.compose.material.CircularProgressIndicator
import androidx.compose.material.Colors
import androidx.compose.material.Divider
import androidx.compose.material.DropdownMenu
import androidx.compose.material.DropdownMenuItem
import androidx.compose.material.Icon
import androidx.compose.material.IconButton
import androidx.compose.material.MaterialTheme
import androidx.compose.material.OutlinedTextField
import androidx.compose.material.OutlinedButton
import androidx.compose.material.Scaffold
import androidx.compose.material.Surface
import androidx.compose.material.Text
import androidx.compose.material.TextButton
import androidx.compose.material.TopAppBar
import androidx.compose.material.Typography
import androidx.compose.material.darkColors
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ArrowDropDown
import androidx.compose.material.icons.filled.Check
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.lightColors
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlin.math.roundToInt

private val Accent = Color(0xFF138A5B)
private val AccentDark = Color(0xFF54D59B)
private val Warning = Color(0xFFE0A11A)
private val Error = Color(0xFFC63C4D)
private val LightBackground = Color(0xFFF5F8F6)
private val DarkBackground = Color(0xFF101513)

@Composable
fun LanPulseApp(client: LanPulseClient) {
    val scope = rememberCoroutineScope()
    val controller = remember(client, scope) { LanPulseController(client, scope) }
    val state by controller.state.collectAsState()
    val dark = isSystemInDarkTheme()
    val strings = state.language.strings()

    LaunchedEffect(controller) {
        controller.discover()
    }

    MaterialTheme(
        colors = lanPulseColors(dark),
        typography = lanPulseTypography(),
    ) {
        Scaffold(
            backgroundColor = MaterialTheme.colors.background,
            topBar = {
                AppBar(
                    playback = state.playback,
                    language = state.language,
                    strings = strings,
                    onLanguageSelected = controller::selectLanguage,
                )
            },
        ) { scaffoldPadding ->
            Box(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(scaffoldPadding),
                contentAlignment = Alignment.TopCenter,
            ) {
                LazyColumn(
                    modifier = Modifier
                        .fillMaxHeight()
                        .widthIn(max = 680.dp)
                        .fillMaxWidth()
                        .navigationBarsPadding(),
                    contentPadding = PaddingValues(horizontal = 20.dp, vertical = 20.dp),
                    verticalArrangement = Arrangement.spacedBy(20.dp),
                ) {
                    state.error?.let { error ->
                        item(key = "error") {
                            ErrorBanner(error, controller::clearError, strings)
                        }
                    }

                    item(key = "computers-header") {
                        SectionHeader(
                            title = strings.computers,
                            searching = state.discoveryState == DiscoveryState.Searching,
                            refreshDescription = strings.refreshComputers,
                            onRefresh = controller::discover,
                        )
                    }

                    items(state.desktops, key = DesktopEndpoint::id) { desktop ->
                        DesktopRow(
                            desktop = desktop,
                            selected = desktop.id == state.selectedDesktopId,
                            strings = strings,
                            onSelect = { controller.selectDesktop(desktop.id) },
                        )
                    }

                    if (
                        state.discoveryState == DiscoveryState.Complete &&
                        state.desktops.isEmpty()
                    ) {
                        item(key = "empty") {
                            Text(
                                text = strings.noComputersFound,
                                style = MaterialTheme.typography.body2,
                                color = MaterialTheme.colors.onSurface.copy(alpha = 0.64f),
                            )
                        }
                    }

                    item(key = "pairing") {
                        PairingForm(state, controller, strings)
                    }

                    if (state.playback !is PlaybackState.Idle) {
                        item(key = "playback") {
                            PlaybackPanel(state.playback, strings, controller::disconnect)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun AppBar(
    playback: PlaybackState,
    language: MobileLanguage,
    strings: MobileStrings,
    onLanguageSelected: (MobileLanguage) -> Unit,
) {
    TopAppBar(
        modifier = Modifier.statusBarsPadding(),
        title = {
            Text(
                text = "LanPulse",
                style = MaterialTheme.typography.h6,
                fontWeight = FontWeight.SemiBold,
            )
        },
        actions = {
            StatusBadge(playback, strings)
            LanguageMenu(language, strings, onLanguageSelected)
            Spacer(Modifier.size(12.dp))
        },
        backgroundColor = MaterialTheme.colors.surface,
        contentColor = MaterialTheme.colors.onSurface,
        elevation = 0.dp,
    )
}

@Composable
private fun StatusBadge(playback: PlaybackState, strings: MobileStrings) {
    val (label, color) = when (playback) {
        PlaybackState.Idle -> strings.ready to MaterialTheme.colors.onSurface.copy(alpha = 0.56f)
        is PlaybackState.Connecting -> strings.connecting to Warning
        is PlaybackState.Playing -> strings.playing to Accent
        is PlaybackState.Reconnecting -> strings.reconnecting to Warning
        is PlaybackState.Failed -> strings.error to Error
    }
    Surface(
        color = color.copy(alpha = 0.12f),
        shape = RoundedCornerShape(6.dp),
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 10.dp, vertical = 7.dp),
            horizontalArrangement = Arrangement.spacedBy(7.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Box(
                Modifier
                    .size(7.dp)
                    .background(color, CircleShape),
            )
            Text(label, color = color, style = MaterialTheme.typography.caption)
        }
    }
}

@Composable
private fun LanguageMenu(
    language: MobileLanguage,
    strings: MobileStrings,
    onSelected: (MobileLanguage) -> Unit,
) {
    var expanded by remember { mutableStateOf(false) }
    Box {
        TextButton(onClick = { expanded = true }) {
            Text(language.displayName)
            Icon(Icons.Default.ArrowDropDown, contentDescription = strings.language)
        }
        DropdownMenu(
            expanded = expanded,
            onDismissRequest = { expanded = false },
        ) {
            for (value in MobileLanguage.entries) {
                DropdownMenuItem(
                    onClick = {
                        expanded = false
                        onSelected(value)
                    },
                ) {
                    Box(Modifier.size(24.dp), contentAlignment = Alignment.Center) {
                        if (value == language) {
                            Icon(Icons.Default.Check, contentDescription = null)
                        }
                    }
                    Spacer(Modifier.size(8.dp))
                    Text(value.displayName)
                }
            }
        }
    }
}

@Composable
private fun SectionHeader(
    title: String,
    searching: Boolean,
    refreshDescription: String,
    onRefresh: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .height(40.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = title,
            modifier = Modifier.weight(1f),
            style = MaterialTheme.typography.subtitle1,
            fontWeight = FontWeight.SemiBold,
        )
        if (searching) {
            CircularProgressIndicator(
                modifier = Modifier.size(22.dp),
                color = MaterialTheme.colors.primary,
                strokeWidth = 2.dp,
            )
        } else {
            IconButton(onClick = onRefresh, modifier = Modifier.size(40.dp)) {
                Icon(Icons.Default.Refresh, contentDescription = refreshDescription)
            }
        }
    }
}

@Composable
private fun DesktopRow(
    desktop: DesktopEndpoint,
    selected: Boolean,
    strings: MobileStrings,
    onSelect: () -> Unit,
) {
    val border = if (selected) Accent else MaterialTheme.colors.onSurface.copy(alpha = 0.14f)
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .selectable(
                selected = selected,
                onClick = onSelect,
                role = Role.RadioButton,
            ),
        shape = RoundedCornerShape(8.dp),
        backgroundColor = MaterialTheme.colors.surface,
        border = androidx.compose.foundation.BorderStroke(if (selected) 1.5.dp else 1.dp, border),
        elevation = 0.dp,
    ) {
        Column(
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
            verticalArrangement = Arrangement.spacedBy(5.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = desktop.name,
                    modifier = Modifier.weight(1f),
                    style = MaterialTheme.typography.body1,
                    fontWeight = FontWeight.Medium,
                )
                if (selected) {
                    Text(
                        strings.selected,
                        color = Accent,
                        style = MaterialTheme.typography.caption,
                    )
                }
            }
            Text(
                text = desktop.controlUrl,
                color = MaterialTheme.colors.onSurface.copy(alpha = 0.62f),
                style = MaterialTheme.typography.caption,
            )
            desktop.audio?.let { audio ->
                Text(
                    text = "${audio.sampleRate / 1000} kHz  /  ${audio.channels} ch  /  ${audio.packetMs} ms",
                    color = MaterialTheme.colors.onSurface.copy(alpha = 0.62f),
                    style = MaterialTheme.typography.caption,
                )
            }
        }
    }
}

@Composable
private fun PairingForm(
    state: MobileUiState,
    controller: LanPulseController,
    strings: MobileStrings,
) {
    Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
        Text(
            text = strings.pairing,
            style = MaterialTheme.typography.subtitle1,
            fontWeight = FontWeight.SemiBold,
        )
        OutlinedButton(
            onClick = controller::scanPairingCode,
            enabled = state.playback is PlaybackState.Idle || state.playback is PlaybackState.Failed,
            modifier = Modifier
                .fillMaxWidth()
                .height(46.dp),
            shape = RoundedCornerShape(6.dp),
            elevation = ButtonDefaults.elevation(defaultElevation = 0.dp, pressedElevation = 0.dp),
        ) {
            Icon(Icons.Default.Search, contentDescription = null)
            Spacer(Modifier.size(8.dp))
            Text(strings.scanPairingCode)
        }
        OutlinedTextField(
            value = state.manualUrl,
            onValueChange = controller::updateManualUrl,
            modifier = Modifier.fillMaxWidth(),
            label = { Text(strings.manualAddress) },
            placeholder = { Text(strings.manualAddressHint) },
            singleLine = true,
            keyboardOptions = KeyboardOptions(
                keyboardType = KeyboardType.Uri,
                imeAction = ImeAction.Next,
            ),
        )
        OutlinedTextField(
            value = state.pin,
            onValueChange = controller::updatePin,
            modifier = Modifier.fillMaxWidth(),
            label = { Text(strings.pinLabel) },
            singleLine = true,
            visualTransformation = PasswordVisualTransformation(),
            keyboardOptions = KeyboardOptions(
                keyboardType = KeyboardType.NumberPassword,
                imeAction = ImeAction.Done,
            ),
        )
        Button(
            onClick = controller::connect,
            enabled = state.playback is PlaybackState.Idle || state.playback is PlaybackState.Failed,
            modifier = Modifier
                .fillMaxWidth()
                .height(48.dp),
            shape = RoundedCornerShape(6.dp),
            colors = ButtonDefaults.buttonColors(backgroundColor = MaterialTheme.colors.primary),
            elevation = ButtonDefaults.elevation(defaultElevation = 0.dp, pressedElevation = 0.dp),
        ) {
            Icon(Icons.Default.PlayArrow, contentDescription = null)
            Spacer(Modifier.size(8.dp))
            Text(strings.connect)
        }
    }
}

@Composable
private fun PlaybackPanel(
    playback: PlaybackState,
    strings: MobileStrings,
    onDisconnect: () -> Unit,
) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        shape = RoundedCornerShape(8.dp),
        backgroundColor = MaterialTheme.colors.surface,
        border = androidx.compose.foundation.BorderStroke(
            1.dp,
            MaterialTheme.colors.onSurface.copy(alpha = 0.14f),
        ),
        elevation = 0.dp,
    ) {
        Column(
            modifier = Modifier.padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(14.dp),
        ) {
            Text(playbackTitle(playback, strings), fontWeight = FontWeight.SemiBold)
            if (playback is PlaybackState.Playing) {
                Divider(color = MaterialTheme.colors.onSurface.copy(alpha = 0.1f))
                Row(Modifier.fillMaxWidth()) {
                    Metric(strings.packets, playback.packetsReceived.toString(), Modifier.weight(1f))
                    Metric(strings.lost, playback.packetsLost.toString(), Modifier.weight(1f))
                    Metric(strings.buffer, "${playback.bufferMs} ms", Modifier.weight(1f))
                }
                Row(Modifier.fillMaxWidth()) {
                    Metric(strings.queued, "${playback.queuedMs} ms", Modifier.weight(1f))
                    Metric(strings.jitter, formatJitter(playback.jitterMs), Modifier.weight(1f))
                    Metric(strings.underruns, playback.audioUnderruns.toString(), Modifier.weight(1f))
                }
            }
            TextButton(onClick = onDisconnect, modifier = Modifier.align(Alignment.End)) {
                Icon(Icons.Default.Close, contentDescription = null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.size(6.dp))
                Text(strings.disconnect)
            }
        }
    }
}

@Composable
private fun Metric(label: String, value: String, modifier: Modifier) {
    Column(modifier, verticalArrangement = Arrangement.spacedBy(3.dp)) {
        Text(value, style = MaterialTheme.typography.body1, fontWeight = FontWeight.Medium)
        Text(
            label,
            style = MaterialTheme.typography.caption,
            color = MaterialTheme.colors.onSurface.copy(alpha = 0.58f),
        )
    }
}

private fun formatJitter(jitterMs: Double): String {
    val roundedTenths = (jitterMs * 10.0).roundToInt() / 10.0
    return "$roundedTenths ms"
}

@Composable
private fun ErrorBanner(message: String, onDismiss: () -> Unit, strings: MobileStrings) {
    Surface(color = Error.copy(alpha = 0.11f), shape = RoundedCornerShape(6.dp)) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(start = 14.dp, top = 10.dp, bottom = 10.dp, end = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = message,
                modifier = Modifier.weight(1f),
                color = Error,
                style = MaterialTheme.typography.body2,
            )
            IconButton(onClick = onDismiss, modifier = Modifier.size(36.dp)) {
                Icon(Icons.Default.Close, contentDescription = strings.dismiss, tint = Error)
            }
        }
    }
}

private fun playbackTitle(playback: PlaybackState, strings: MobileStrings): String = when (playback) {
    PlaybackState.Idle -> strings.ready
    is PlaybackState.Connecting -> "${strings.connectingTo} ${playback.desktopName}"
    is PlaybackState.Playing -> "${strings.playingFrom} ${playback.desktopName}"
    is PlaybackState.Reconnecting -> "${strings.reconnectingTo} ${playback.desktopName}"
    is PlaybackState.Failed -> playback.message
}

@Composable
private fun lanPulseColors(dark: Boolean): Colors = if (dark) {
    darkColors(
        primary = AccentDark,
        primaryVariant = Accent,
        secondary = Warning,
        background = DarkBackground,
        surface = Color(0xFF18201D),
        error = Error,
        onPrimary = Color(0xFF062A1B),
        onBackground = Color(0xFFE8EFEB),
        onSurface = Color(0xFFE8EFEB),
    )
} else {
    lightColors(
        primary = Accent,
        primaryVariant = Color(0xFF0D6944),
        secondary = Warning,
        background = LightBackground,
        surface = Color.White,
        error = Error,
        onPrimary = Color.White,
        onBackground = Color(0xFF17201C),
        onSurface = Color(0xFF17201C),
    )
}

private fun lanPulseTypography(): Typography {
    val family = FontFamily.SansSerif
    return Typography(
        h4 = TextStyle(
            fontFamily = family,
            fontWeight = FontWeight.SemiBold,
            fontSize = 28.sp,
            lineHeight = 34.sp,
            letterSpacing = 0.sp,
        ),
        h6 = TextStyle(
            fontFamily = family,
            fontWeight = FontWeight.SemiBold,
            fontSize = 20.sp,
            lineHeight = 26.sp,
            letterSpacing = 0.sp,
        ),
        subtitle1 = TextStyle(
            fontFamily = family,
            fontWeight = FontWeight.Medium,
            fontSize = 17.sp,
            lineHeight = 24.sp,
            letterSpacing = 0.sp,
        ),
        body1 = TextStyle(
            fontFamily = family,
            fontWeight = FontWeight.Normal,
            fontSize = 16.sp,
            lineHeight = 23.sp,
            letterSpacing = 0.sp,
        ),
        body2 = TextStyle(
            fontFamily = family,
            fontWeight = FontWeight.Normal,
            fontSize = 14.sp,
            lineHeight = 20.sp,
            letterSpacing = 0.sp,
        ),
        button = TextStyle(
            fontFamily = family,
            fontWeight = FontWeight.SemiBold,
            fontSize = 14.sp,
            lineHeight = 20.sp,
            letterSpacing = 0.sp,
        ),
        caption = TextStyle(
            fontFamily = family,
            fontWeight = FontWeight.Normal,
            fontSize = 12.sp,
            lineHeight = 17.sp,
            letterSpacing = 0.sp,
        ),
    )
}
