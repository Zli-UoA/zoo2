package zoo

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import kotlin.math.ceil
import kotlinx.serialization.json.*

/** Samples live beside the shared UI so native catalogs can render the same examples. */
object CatalogSamples {
    const val message = "こんにちは。今日のリリースは 18:00 です。詳細は https://example.com/docs を参照してください。"
    const val richText =
        "**Compose Multiplatform**\n\n$[tada Hello!] :smile: **太字**\n\n\\[x^2 + y^2 = z^2\\]\n\n```kotlin\nval greeting = \"こんにちは\"\n```"
    val users = listOf(obj("id" to "alice", "display_name" to "Alice", "user_name" to "alice"))
    val channel = obj("id" to "catalog", "channel_name" to "general", "created_user_id" to "alice")
}

@Composable
fun Catalog(app: Session, options: Row) {
    val kind = options.text("sample")
    val fixture = options.nested("fixture")
    fun callback(name: String, args: JsonArray = JsonArray(emptyList())) {
        if (fixture.array("callbacks").any { it.jsonPrimitive.content == name })
            app.run {
                app.platform.invoke("storyCallback", obj("name" to name, "args" to args).toString())
            }
    }
    val users = (fixture["users"] as? JsonArray)?.map { it.jsonObject } ?: CatalogSamples.users
    val density = LocalDensity.current
    var measuredHeight by remember { mutableStateOf(0f) }
    var measuredWidth by remember { mutableStateOf(0f) }
    var hasMeasured by remember { mutableStateOf(false) }
    LaunchedEffect(hasMeasured, measuredWidth, measuredHeight) {
        if (hasMeasured) app.platform.action("catalog-size:$measuredWidth:$measuredHeight")
    }
    val overlayOpen = app.overlays.any { it.visible } || app.toastState != null
    // Re-announce after controlled args change as well as open/close. The host
    // must receive the current clipping state when an existing preview reopens.
    LaunchedEffect(overlayOpen, fixture) { app.platform.action("catalog-overlay:$overlayOpen") }
    val overlayBounds = options.nested("overlayBounds")
    val normalBounds = options.nested("normalBounds")
    val bounds = if (normalBounds.isNotEmpty()) normalBounds else overlayBounds
    val surface =
        if (bounds.isEmpty()) Modifier
        else {
            val width = bounds.text("width").toFloat().dp
            val widthPixels = with(density) { width.toPx() }
            val constraintWidth = with(density) { ceil(widthPixels).toDp() }
            Modifier.graphicsLayer {
                    translationX = with(density) { bounds.text("x").toFloat().dp.toPx() }
                    translationY = with(density) { bounds.text("y").toFloat().dp.toPx() }
                }
                .cssPreferredWidth(widthPixels)
                .then(
                    if (options.flag("measureWidth")) Modifier.widthIn(max = constraintWidth)
                    else Modifier.width(constraintWidth)
                )
                .then(
                    if (options.flag("fullHeight"))
                        Modifier.height(bounds.text("height").toFloat().dp)
                    else Modifier
                )
        }

    val content =
        if ("content" in options) options.text("content")
        else if (kind == "rich-text") CatalogSamples.richText else CatalogSamples.message
    var selected by remember { mutableStateOf("") }
    var picker by remember(kind) { mutableStateOf(kind == "emoji-picker") }
    NativeWindow(
        app,
        Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).focusInputModality(),
    ) {
        Column(
            surface
                .then(
                    if (options.flag("fullHeight")) Modifier.fillMaxSize()
                    else if (options.flag("measureWidth"))
                        Modifier.width(IntrinsicSize.Max).wrapContentHeight(unbounded = true)
                    else Modifier.fillMaxWidth().wrapContentHeight(unbounded = true)
                )
                .onSizeChanged { size ->
                    hasMeasured = true
                    measuredHeight = with(density) { size.height.toDp().value }
                    measuredWidth = with(density) { size.width.toDp().value }
                },
            horizontalAlignment =
                if (kind == "dialog-demo") Alignment.CenterHorizontally else Alignment.Start,
        ) {
            when (kind) {
                "primitive-story" -> PrimitiveStory(app, options)
                "message-story" -> MessageStory(app, options)
                "shell-story" -> ShellStory(app, options)
                "app-story" -> AppStory(app, options)
                "preference-story" -> PreferenceStory(app, options)
                "notification-story" -> NotificationStory(app, options)
                "home-story" -> HomeScreen(app, fixture, fixture.nested("args"))
                "dialog-demo",
                "confirm" -> {
                    var open by
                        remember(fixture.flag("open")) { mutableStateOf(fixture.flag("open")) }
                    if (kind == "dialog-demo")
                        Button(fixture.text("trigger"), variant = "outline") { open = true }
                    if (open && kind == "confirm")
                        ConfirmationPanel(
                            fixture.text("title"),
                            fixture.text("description"),
                            fixture.text("confirm"),
                            fixture.flag("pending"),
                            fixture.text("error"),
                            { callback("onOpenChange", JsonArray(listOf(JsonPrimitive(false)))) },
                            { callback("onConfirm") },
                        )
                    else if (open)
                        Panel(
                            fixture.text("title"),
                            { open = false },
                            alert = fixture.flag("alert"),
                            description = fixture.text("description"),
                            footer = {
                                DialogFooter(
                                    listOf(
                                        DialogAction(
                                            fixture.text("cancel"),
                                            action = { open = false },
                                        ),
                                        DialogAction(
                                            fixture.text("confirm"),
                                            "destructive",
                                            action = { open = false },
                                        ),
                                    )
                                )
                            },
                        ) {}
                }
                "control-panel" ->
                    SettingsSection(fixture.text("title"), fixture.text("description")) {
                        SettingRow(fixture.text("rowLabel"), fixture.text("rowDescription")) {
                            Button(fixture.text("actionLabel"), variant = "outline") {}
                        }
                    }
                "select" -> {
                    var selected by
                        remember(fixture.text("value")) { mutableStateOf(fixture.text("value")) }
                    SelectField(
                        fixture.text("ariaLabel"),
                        selected,
                        fixture.array("options").map { it.jsonPrimitive.content },
                        enabled = !options.flag("disabled"),
                        allowEmpty = false,
                    ) {
                        if (!fixture.flag("controlled")) selected = it
                        val selectedIndex =
                            fixture.array("options").indexOfFirst { choice ->
                                choice.jsonPrimitive.content == it
                            }
                        val actualValue =
                            fixture
                                .array("optionValues")
                                .getOrNull(selectedIndex)
                                ?.jsonPrimitive
                                ?.content ?: it
                        callback(
                            "onChange",
                            JsonArray(
                                listOf(
                                    obj(
                                        "target" to obj("value" to actualValue),
                                        "currentTarget" to obj("value" to actualValue),
                                    )
                                )
                            ),
                        )
                    }
                }
                "avatar" -> {
                    val initials = fixture.nested("avatarInitials")
                    val labels = initials.array("sizes").map { it.jsonPrimitive.content }
                    val size =
                        when (fixture.text("size")) {
                            "sm" -> 24.dp
                            "lg" -> 40.dp
                            else -> 32.dp
                        }
                    when (fixture.text("state")) {
                        "Sizes" ->
                            Row(
                                horizontalArrangement = Arrangement.spacedBy(12.dp),
                                verticalAlignment = Alignment.Bottom,
                            ) {
                                Avatar(null, size = 24.dp, fallback = labels[0])
                                Avatar(null, size = 32.dp, fallback = labels[1])
                                Avatar(null, size = 40.dp, fallback = labels[2])
                            }
                        "Group" ->
                            Row(horizontalArrangement = Arrangement.spacedBy((-8).dp)) {
                                val ring = MaterialTheme.colorScheme.background
                                val groupedAvatar =
                                    Modifier.drawBehind {
                                        drawCircle(
                                            ring,
                                            radius = this.size.minDimension / 2 + 2.dp.toPx(),
                                        )
                                    }
                                initials
                                    .array("group")
                                    .map { it.jsonPrimitive.content }
                                    .forEach {
                                        Avatar(
                                            null,
                                            modifier = groupedAvatar,
                                            size = 24.dp,
                                            fallback = it,
                                        )
                                    }
                                Box(
                                    groupedAvatar
                                        .size(24.dp)
                                        .background(
                                            MaterialTheme.colorScheme.surfaceVariant,
                                            CircleShape,
                                        ),
                                    contentAlignment = Alignment.Center,
                                ) {
                                    Text(initials.text("count"), muted = true)
                                }
                            }
                        "Badge" -> Avatar(null, fallback = initials.text("badge"), badge = true)
                        else -> Avatar(null, size = size, fallback = initials.text("default"))
                    }
                }
                "skeleton" -> {
                    val avatar = fixture.text("state") == "Avatar"
                    SkeletonBlock(
                        Modifier.size(if (avatar) 40.dp else 256.dp, if (avatar) 40.dp else 16.dp),
                        radius = if (avatar) 999.dp else 8.dp,
                    )
                }
                "button" -> {
                    CssFlowRow(
                        horizontalGap = 8.dp,
                        verticalGap = 8.dp,
                        verticalAlignment =
                            if (fixture.text("state") == "Sizes") Alignment.Bottom
                            else Alignment.CenterVertically,
                    ) {
                        when (fixture.text("state")) {
                            "Variants" ->
                                listOf(
                                        "default",
                                        "outline",
                                        "secondary",
                                        "ghost",
                                        "destructive",
                                        "link",
                                    )
                                    .forEach { variant -> Button(variant, variant = variant) {} }
                            "Sizes" -> {
                                listOf("xs", "sm", "default", "lg").forEach { size ->
                                    Button(size, variant = "default", size = size) {}
                                }
                                Button("追加", variant = "default", size = "icon", icon = "plus") {}
                                Button(
                                    "ダウンロード",
                                    variant = "default",
                                    size = "icon-sm",
                                    icon = "download",
                                ) {}
                            }
                            else ->
                                Button(
                                    fixture.text("label"),
                                    enabled = !options.flag("disabled"),
                                    variant = fixture.text("variant"),
                                    size = fixture.text("size"),
                                    icon = fixture.text("icon").ifEmpty { null },
                                    inlineStartIcon = fixture.text("state") == "With Icon",
                                ) {
                                    callback("onClick")
                                }
                        }
                    }
                }
                "input",
                "textarea" -> {
                    var value by
                        remember(fixture.text("value")) { mutableStateOf(fixture.text("value")) }
                    Field(
                        value,
                        fixture.text("ariaLabel"),
                        singleLine = kind == "input",
                        showLabel = false,
                        enabled = !options.flag("disabled"),
                        placeholder = fixture.text("placeholder"),
                        invalid = fixture.flag("invalid"),
                        readOnly = fixture.flag("readOnly"),
                    ) {
                        if (!fixture.flag("controlled")) value = it
                        callback(
                            "onChange",
                            JsonArray(
                                listOf(
                                    obj(
                                        "target" to obj("value" to it),
                                        "currentTarget" to obj("value" to it),
                                    )
                                )
                            ),
                        )
                    }
                }
                "card" -> MessageCard(fixture.nested("card"), fixture.flag("attachment"))
                "app-blocks" ->
                    Blocks(
                        app,
                        fixture["blocks"],
                        fixture.text("appId"),
                        submit = fixture.flag("submit"),
                    )
                "link-preview" -> LinkPreviews(app, content)

                "settings",
                "settings-members" -> ZooSettings(app, users)
                "bubble" -> BubbleChat(app, users)
                "notifications",
                "notifications-loading" ->
                    Notifications(
                        app,
                        (fixture["notifications"] as? JsonArray)?.map { it.jsonObject }
                            ?: listOf(
                                obj(
                                    "id" to "catalog-notification",
                                    "actor_id" to "alice",
                                    "user_id" to "alice",
                                    "channel_id" to "catalog",
                                    "message_id" to "catalog-message",
                                    "notification_type" to "mention",
                                    "created_at" to "2026-01-02T09:00:00Z",
                                    "is_read" to "true",
                                )
                            ),
                        users,
                    )
                "composer" -> Composer(app, "catalog", null, users, !options.flag("disabled"))
                "emoji-picker" ->
                    Column(Modifier.padding(16.dp)) {
                        Button(app.t("reactions.add")) { picker = true }
                        Text(selected)
                    }
                "rich-text" -> Column(Modifier.fillMaxWidth()) { RichText(content, users) }
                else -> {
                    val message =
                        JsonObject(
                            obj(
                                "id" to "catalog-message",
                                "user_id" to "alice",
                                "content" to content,
                                "created_at" to "2026-01-02T09:00:00Z",
                            ) + options.nested("fixture")
                        )
                    val attachmentRows =
                        (options.nested("fixture")["attachments"] as? JsonArray)
                            ?.map { it.jsonObject }
                            .orEmpty()
                    CompositionLocalProvider(LocalAttachments provides attachmentRows) {
                        Message(
                            app,
                            CatalogSamples.channel,
                            message,
                            listOf(message) +
                                List(fixture.text("replyCount").toIntOrNull() ?: 0) { index ->
                                    obj("id" to "reply-$index", "parent_id" to message.text("id"))
                                },
                            fixture.array("reactions").map { it.jsonObject },
                            users,
                            options.flag("grouped"),
                            true,
                        )
                    }
                }
            }
        }
        if (picker) EmojiPicker(app, { picker = false }) { selected = it }
    }
}
