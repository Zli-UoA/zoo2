package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import kotlinx.serialization.json.*

/** One draft survives closing the composer and switching between a post and a reply. */
@Stable
class HomeComposerState(internal val settingsKey: String? = null) {
    internal var focusOnOpen: (() -> Unit)? = null
    internal var preferencesReady by mutableStateOf(settingsKey == null)

    var opened by mutableStateOf(false)
        internal set

    var content by mutableStateOf("")
        internal set

    var visibility by mutableStateOf("unlisted")
        internal set

    var federationPreference by mutableStateOf<Boolean?>(null)
        internal set

    var channelId by mutableStateOf("")
        internal set

    var replyTarget by mutableStateOf<Row?>(null)
        internal set

    var sending by mutableStateOf(false)
        internal set

    var failed by mutableStateOf(false)
        internal set

    fun open() {
        if (!sending && preferencesReady) {
            opened = true
            focusOnOpen?.invoke()
        }
    }

    fun close() {
        if (!sending) opened = false
    }

    fun reply(post: Row) {
        if (sending || !preferencesReady) return
        replyTarget = post
        channelId = ""
        visibility = homeReplyVisibility(visibility, post.text("visibility"))
        opened = true
        focusOnOpen?.invoke()
    }
}

@Composable fun rememberHomeComposerState(): HomeComposerState {
    val app = LocalApp.current
    val key = homePreferenceKey(app, "zoo-home-post-settings:v1:")
    val state = remember(app, key) { HomeComposerState(key) }
    LaunchedEffect(state) {
        val saved = parseHomePostSettings(readHomePreference(app, key))
        state.visibility = saved.text("visibility")
        state.federationPreference = saved.flag("apub")
        state.channelId = saved.text("channel_id")
        state.preferencesReady = true
    }
    return state
}

internal fun homePostLength(content: String): Int {
    var count = 0
    var index = 0
    while (index < content.length) {
        val char = content[index++]
        if (char.isHighSurrogate() && index < content.length && content[index].isLowSurrogate())
            index++
        count++
    }
    return count
}

@Composable
fun HomeComposer(
    app: Session,
    federationEnabled: Boolean,
    state: HomeComposerState = rememberHomeComposerState(),
    modifier: Modifier = Modifier,
    onPost: suspend (Row) -> Unit,
) {
    val desktop = LocalViewportWidth.current >= 768f
    val scope = rememberCoroutineScope()
    val channels by app.repo.channels.rows.collectAsState()
    val memberships by app.repo.members.rows.collectAsState()
    val joined =
        memberships
            .filter { it.text("user_id") == app.config.userId }
            .map { it.text("channel_id") }
            .toSet()
    val available =
        channels.filter {
            !it.flag("is_private") &&
                !it.flag("is_dm") &&
                !it.present("archive_at") &&
                it.text("id") in joined
        }
    val selectedChannel = state.channelId.takeIf { id -> state.visibility != "followers" && available.any { it.text("id") == id } }.orEmpty()
    val reply = state.replyTarget
    val federated =
        federationEnabled &&
            reply?.get("apub") != JsonPrimitive(false) &&
            (state.federationPreference ?: true)
    val length = homePostLength(state.content)
    val inputFocus = remember { FocusRequester() }
    DisposableEffect(state, inputFocus) {
        state.focusOnOpen = { inputFocus.requestFocus() }
        onDispose { state.focusOnOpen = null }
    }
    val latestPost by rememberUpdatedState(onPost)
    val colors = MaterialTheme.colorScheme
    Box(
        modifier
            .size(56.dp)
            .clip(CircleShape)
            .background(colors.primary)
            .then(
                webControlModifier(
                    "button",
                    attributes = mapOf("aria-label" to app.t("home.openComposer"), "aria-disabled" to (!state.preferencesReady).toString()),
                )
            )
            .clickable(enabled = state.preferencesReady, role = Role.Button) { state.open() }
            .semantics { contentDescription = app.t("home.openComposer") },
        contentAlignment = Alignment.Center,
    ) {
        Icon("square-pen", modifier = Modifier.size(24.dp), color = colors.onPrimary)
    }
    OverlayRegistration(
        app.t("home.compose"),
        state::close,
        576.dp,
        visible = state.opened,
        focusEnabled = { state.opened },
        customChrome = true,
        fullScreen = !desktop,
        onEscape = {
            state.close()
            true
        },
    ) {
        Column(if (desktop) Modifier.fillMaxWidth() else Modifier.fillMaxSize()) {
            Row(
                Modifier.fillMaxWidth()
                    .background(colors.surface)
                    .padding(horizontal = 12.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                IconButton(
                    if (desktop) "x" else "arrow-left",
                    app.t("common.close"),
                    !state.sending,
                ) {
                    state.close()
                }
                Text(
                    app.t("home.compose"),
                    Modifier.weight(1f)
                        .then(
                            webControlModifier("heading", attributes = mapOf("aria-level" to "2"))
                        ),
                    weight = FontWeight.Medium,
                    maxLines = 1,
                )
                Row(
                    Modifier.then(
                        webControlModifier(
                            "group",
                            attributes = mapOf("aria-label" to app.t("home.postOptions")),
                        )
                    ),
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    val audienceIcon =
                        when (state.visibility) {
                            "public" -> "globe"
                            "followers" -> "lock-keyhole"
                            else -> "home"
                        }
                    HomePostMenu(
                        "${app.t("home.audience")}: ${app.t("home.visibility.${state.visibility}")}",
                        audienceIcon,
                        !state.sending,
                        visible = state.opened,
                    ) { close ->
                        listOf(
                                "public" to "globe",
                                "unlisted" to "home",
                                "followers" to "lock-keyhole",
                            )
                            .forEach { (value, icon) ->
                                val enabled =
                                    !(reply?.text("visibility") == "followers" &&
                                        value != "followers") &&
                                        !(reply?.text("visibility") == "unlisted" &&
                                            value == "public")
                                HomePostChoice(
                                    app.t("home.visibility.$value"),
                                    icon,
                                    state.visibility == value,
                                    enabled,
                                ) {
                                    state.visibility = value
                                    if (value == "followers") state.channelId = ""
                                    close()
                                }
                            }
                    }
                    HomePostMenu(
                        app.t(if (federated) "home.federationOn" else "home.federationOff"),
                        if (federated) "network" else "unplug",
                        !state.sending,
                        visible = state.opened,
                    ) { close ->
                        HomePostChoice(
                            app.t("home.federationOn"),
                            "network",
                            federated,
                            federationEnabled && reply?.get("apub") != JsonPrimitive(false),
                        ) {
                            state.federationPreference = true
                            close()
                        }
                        HomePostChoice(app.t("home.federationOff"), "unplug", !federated, true) {
                            state.federationPreference = false
                            close()
                        }
                        if (!federationEnabled)
                            Text(
                                app.t("home.federationDisabled"),
                                Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
                                small = true,
                                muted = true,
                            )
                    }
                    Button(
                        "",
                        enabled = !state.sending && state.content.isNotBlank() && length <= 1000,
                        primary = true,
                        size = "icon",
                        icon = if (state.sending) "loader-circle" else "send",
                        accessibleLabel = app.t(if (state.sending) "home.sending" else "home.post"),
                    ) {
                        if (!state.sending && state.content.isNotBlank() && length <= 1000) {
                            state.sending = true
                            state.failed = false
                            val payload = buildJsonObject {
                                put("content", state.content)
                                put("apub", federated)
                                put("visibility", state.visibility)
                                put(
                                    "channel_id",
                                    selectedChannel.takeIf { it.isNotBlank() }?.let(::JsonPrimitive)
                                        ?: JsonNull,
                                )
                                state.replyTarget?.let { put("in_reply_to", it.text("id")) }
                            }
                            scope.launch {
                                try {
                                    latestPost(payload)
                                    writeHomePreference(app, state.settingsKey, obj("visibility" to state.visibility, "apub" to federated, "channel_id" to selectedChannel.takeIf { it.isNotBlank() }).toString())
                                    state.federationPreference = federated
                                    state.channelId = selectedChannel
                                    state.replyTarget = null
                                    state.content = ""
                                    state.opened = false
                                } catch (cancelled: CancellationException) {
                                    throw cancelled
                                } catch (_: Exception) {
                                    state.failed = true
                                } finally {
                                    state.sending = false
                                }
                            }
                        }
                    }
                }
            }
            HorizontalDivider(color = colors.outline)
            BoxWithConstraints(
                (if (desktop) Modifier.heightIn(max = 600.dp) else Modifier.weight(1f))
                    .fillMaxWidth()
            ) {
                val editorHeight =
                    if (desktop) 192.dp
                    else
                        (maxHeight -
                                76.dp -
                                (if (reply != null) 96.dp else 0.dp) -
                                (if (state.failed) 32.dp else 0.dp))
                            .coerceAtLeast(160.dp)
                Column(
                    Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(16.dp),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    if (reply != null) {
                        Row(
                            Modifier.fillMaxWidth()
                                .background(colors.surfaceVariant, RoundedCornerShape(8.dp))
                                .padding(12.dp),
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                            verticalAlignment = Alignment.Top,
                        ) {
                            Column(
                                Modifier.weight(1f),
                                verticalArrangement = Arrangement.spacedBy(4.dp),
                            ) {
                                ActivityPubText(
                                    app.t("home.replyTo", "name" to reply.text("display_name")),
                                    emojis =
                                        (reply["actor_emojis"] as? JsonArray)
                                            ?.mapNotNull { it as? JsonObject }
                                            .orEmpty(),
                                    textStyle =
                                        MaterialTheme.typography.bodySmall.copy(
                                            fontWeight = FontWeight.Medium
                                        ),
                                )
                                ActivityPubContent(
                                    reply.text("content"),
                                    emojis =
                                        (reply["emojis"] as? JsonArray)
                                            ?.mapNotNull { it as? JsonObject }
                                            .orEmpty(),
                                    textStyle =
                                        MaterialTheme.typography.bodySmall.copy(
                                            color = colors.onSurfaceVariant
                                        ),
                                    maxLines = 2,
                                )
                            }
                            IconButton(
                                "x",
                                app.t("home.cancelReply"),
                                !state.sending,
                                size = 24.dp,
                            ) {
                                state.replyTarget = null
                            }
                        }
                    }
                    Field(
                        state.content,
                        app.t("home.compose"),
                        Modifier.fillMaxWidth().focusProperties { canFocus = state.opened },
                        singleLine = false,
                        chrome = false,
                        showLabel = false,
                        placeholder = app.t("home.placeholder"),
                        enabled = !state.sending,
                        invalid = length > 1000,
                        inputAttributes = mapOf("aria-describedby" to "home-post-count"),
                        focusRequester = inputFocus,
                        autoFocus = state.opened,
                        minimumHeight = editorHeight,
                        maximumHeight = Dp.Infinity,
                        resizable = false,
                        contentPadding = PaddingValues(0.dp),
                    ) {
                        state.content = it
                    }
                    HomeComposerFooter(app, state, available, length)

                    if (state.failed)
                        Text(
                            app.t("home.sendError"),
                            Modifier.then(webControlModifier("alert")),
                            color = colors.error,
                        )
                }
            }
        }
    }
}

@Composable
private fun HomePostMenu(
    label: String,
    icon: String,
    enabled: Boolean,
    visible: Boolean,
    content: @Composable ColumnScope.(() -> Unit) -> Unit,
) {
    // The retained editor must not retain transient portals when its form closes.
    // React unmounts DialogContent on close; reset menus at that same boundary.
    var open by remember(visible) { mutableStateOf(false) }
    var anchor by remember { mutableStateOf(Rect.Zero) }
    IconButton(
        icon,
        label,
        enabled,
        modifier = Modifier.onGloballyPositioned { anchor = it.boundsInRoot() },
    ) {
        open = true
    }
    if (visible && open)
        OverlayRegistration(
            label,
            { open = false },
            240.dp,
            anchor = anchor,
            role = "menu",
            alignEnd = true,
        ) {
            content { open = false }
        }
}

@Composable
private fun HomePostChoice(
    label: String,
    icon: String,
    selected: Boolean,
    enabled: Boolean,
    click: () -> Unit,
) {
    Row(
        Modifier.fillMaxWidth()
            .then(
                webControlModifier(
                    "menuitemradio",
                    enabled,
                    attributes = mapOf("aria-checked" to selected.toString()),
                )
            )
            .clickable(enabled = enabled, role = Role.RadioButton, onClick = click)
            .semantics {
                contentDescription = label
                this.selected = selected
                if (!enabled) disabled()
            }
            .padding(horizontal = 8.dp, vertical = 6.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, modifier = Modifier.size(16.dp))
        Text(
            label,
            Modifier.weight(1f),
            color =
                if (enabled) MaterialTheme.colorScheme.onSurface
                else MaterialTheme.colorScheme.onSurface.copy(alpha = .5f),
        )
        if (selected) Icon("check", modifier = Modifier.size(16.dp))
    }
}

@Composable
private fun HomeComposerFooter(
    app: Session,
    state: HomeComposerState,
    available: List<Row>,
    length: Int,
) {
    val colors = MaterialTheme.colorScheme
    val channel: @Composable () -> Unit = {
        Row(
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(app.t("home.channel"), small = true)
            val labels =
                listOf(app.t("home.noChannel")) + available.map { "#${it.text("channel_name")}" }
            SelectField(
                app.t("home.channel"),
                available
                    .find { it.text("id") == state.channelId }
                    ?.let { "#${it.text("channel_name")}" } ?: labels.first(),
                labels,
                enabled = !state.sending && state.visibility != "followers",
                allowEmpty = false,
            ) { selected ->
                state.channelId =
                    available.find { "#${it.text("channel_name")}" == selected }?.text("id") ?: ""
            }
        }
    }
    val count: @Composable () -> Unit = {
        Text(
            "$length / 1000",
            Modifier.then(
                webControlModifier("generic", attributes = mapOf("id" to "home-post-count"))
            ),
            small = true,
            muted = length <= 1000,
            color =
                if (length > 1000) colors.error else androidx.compose.ui.graphics.Color.Unspecified,
        )
    }
    BoxWithConstraints(Modifier.fillMaxWidth()) {
        if (maxWidth >= 340.dp)
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                channel()
                count()
            }
        else
            Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                channel()
                Box(Modifier.align(Alignment.End)) { count() }
            }
    }
}
