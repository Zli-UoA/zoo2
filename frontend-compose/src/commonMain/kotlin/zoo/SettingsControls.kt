package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.focus.*
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.input.key.*
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*
import kotlinx.serialization.json.jsonObject

@Composable
fun SettingRow(
    label: String,
    description: String = "",
    icon: String? = null,
    content: @Composable RowScope.() -> Unit,
) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 64.dp).padding(16.dp),
        horizontalArrangement = Arrangement.spacedBy(16.dp),
        verticalAlignment = Alignment.Top,
    ) {
        Row(
            Modifier.weight(1f).heightIn(min = 32.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (icon != null) Icon(icon, modifier = Modifier.size(20.dp))
            Column(Modifier.weight(1f)) {
                Text(label, weight = FontWeight.Medium)
                if (description.isNotEmpty())
                    Text(description, Modifier.padding(top = 4.dp), muted = true, small = true)
            }
        }
        Row(
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
            content = content,
        )
    }
}

@Composable
fun SettingsSection(
    title: String,
    description: String = "",
    modifier: Modifier = Modifier,
    borderColor: androidx.compose.ui.graphics.Color? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(
        modifier
            .fillMaxWidth()
            .border(
                1.dp,
                borderColor ?: MaterialTheme.colorScheme.outline,
                RoundedCornerShape(10.dp),
            )
            .padding(17.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (title.isNotEmpty())
            Column {
                Text(title, weight = FontWeight.SemiBold)
                if (description.isNotEmpty())
                    Text(description, Modifier.padding(top = 4.dp), muted = true, small = true)
            }
        content()
    }
}

@Composable
fun ThemeToggle(app: Session) {
    val values = listOf("light", "dark", "system")
    val icons = listOf("sun", "moon", "monitor")
    val focus = remember { List(3) { FocusRequester() } }
    Row(
        Modifier.then(webControlModifier("group"))
            .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(10.dp))
            .background(
                MaterialTheme.colorScheme.surfaceVariant.copy(alpha = .4f),
                RoundedCornerShape(10.dp),
            )
            .padding(3.dp)
            .semantics {
                contentDescription = app.t("theme.title")
                isTraversalGroup = true
            },
        horizontalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        values.forEachIndexed { index, value ->
            Hint(app.t("theme.$value")) { hintModifier ->
                Button(
                    app.t("theme.$value"),
                    variant = "toggle",
                    size = "icon",
                    icon = icons[index],
                    selected = app.env.theme == value,
                    modifier =
                        hintModifier.size(40.dp).focusRequester(focus[index]).onPreviewKeyEvent {
                            if (it.type != KeyEventType.KeyDown) false
                            else {
                                val next =
                                    when (it.key) {
                                        Key.DirectionRight,
                                        Key.DirectionDown -> (index + 1) % 3
                                        Key.DirectionLeft,
                                        Key.DirectionUp -> (index + 2) % 3
                                        Key.MoveHome -> 0
                                        Key.MoveEnd -> 2
                                        else -> -1
                                    }
                                if (next < 0) false
                                else {
                                    focus[next].requestFocus()
                                    true
                                }
                            }
                        },
                ) {
                    app.platform.action("theme:$value")
                }
            }
        }
    }
}

@Composable
fun SearchQuery(app: Session, query: String) {
    val measure = androidx.compose.ui.text.rememberTextMeasurer()
    val density = androidx.compose.ui.platform.LocalDensity.current
    val style = MaterialTheme.typography.bodyMedium
    val fonts = LocalFonts.current
    val desired =
        with(density) { measure.measure(fontText(query, fonts), style).size.width.toDp() } + 18.dp
    val buttonWidth =
        with(density) {
            measure
                .measure(
                    fontText(app.t("search.action"), fonts, FontWeight.Medium),
                    style.copy(fontWeight = FontWeight.Medium),
                )
                .size
                .width
                .toDp()
        } + 44.dp
    BoxWithConstraints(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
        val availableWidth = maxWidth
        Row(
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Field(
                query,
                app.t("search.query"),
                modifier =
                    Modifier.width(
                        minOf(desired, (availableWidth - buttonWidth - 6.dp).coerceAtLeast(1.dp))
                    ),
                readOnly = true,
                showLabel = false,
            ) {}
            val engineLabel = searchEngines[app.searchEngine].orEmpty()
            Button(
                app.t("search.action"),
                variant = "outline",
                icon = "search",
                title = engineLabel,
                accessibleLabel = app.t("search.withEngine", "engine" to engineLabel),
            ) {
                app.platform.action("open:${searchUrl(app.searchEngine, query)}")
            }
        }
    }
}

@Composable
fun PwaControls(app: Session, presentation: String = "icon") {
    if (presentation == "select") {
        PwaSelect(app)
        return
    }
    val enabled = app.capabilities.flag("pushEnabled")
    val supported = app.capabilities.flag("pushSupported")
    val configured = app.capabilities.flag("pushConfigured")
    val blocked = app.capabilities.flag("pushBlocked")
    val busy = app.capabilities.flag("pushBusy") || "push" in app.pending
    val label =
        app.t(
            if (!supported) "pwa.unsupported"
            else if (!enabled && !configured) "pwa.unconfigured"
            else if (!enabled && blocked) "pwa.blocked"
            else if (enabled) "pwa.disable" else "pwa.enable"
        )
    Row(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (app.capabilities.flag("installAvailable"))
            Button(app.t("pwa.install"), variant = "ghost", size = "icon", icon = "download") {
                app.run {
                    app.platform.invoke("install")
                    app.capabilities =
                        kotlinx.serialization.json.JsonObject(
                            app.capabilities +
                                ("installAvailable" to
                                    kotlinx.serialization.json.JsonPrimitive(false))
                        )
                }
            }
        Button(
            label,
            enabled = !busy && supported && (enabled || configured && !blocked),
            variant = "ghost",
            size = "icon",
            icon = if (enabled) "bell" else "bell-off",
            selected = enabled,
            title = label,
        ) {
            app.run("push") {
                app.platform.invoke("push")
                app.capabilities =
                    json
                        .parseToJsonElement(app.platform.invoke("capabilities"))
                        .jsonObject
                        .ifEmpty { app.capabilities }
            }
        }
    }
}

@Composable
fun LanguageToggle(app: Session, presentation: String = "icon", initialOpen: Boolean = false) {
    if (presentation == "select") {
        LanguageSelect(app)
        return
    }
    val languages =
        linkedMapOf(
            "ja" to "日本語",
            "en" to "English",
            "zh-Hant" to "繁體中文",
            "zh-Hans" to "简体中文",
            "yue" to "廣東話",
        )
    var open by remember { mutableStateOf(initialOpen) }
    var anchor by remember { mutableStateOf(Rect.Zero) }
    Box(Modifier.onGloballyPositioned { anchor = it.boundsInRoot() }) {
        Button(
            app.t("language.switch"),
            variant = "ghost",
            size = "icon",
            icon = "languages",
            expanded = open,
        ) {
            open = !open
        }
    }
    if (open)
        OverlayRegistration(
            app.t("language.title"),
            { open = false },
            176.dp,
            anchor,
            alignEnd = true,
            anchorOffset = 4.dp,
        ) {
            Column(Modifier.then(webControlModifier("menu"))) {
                Text(
                    app.t("language.title"),
                    Modifier.padding(horizontal = 6.dp, vertical = 4.dp),
                    small = true,
                    muted = true,
                    weight = FontWeight.Medium,
                )
                androidx.compose.material3.HorizontalDivider(Modifier.padding(vertical = 4.dp))
                languages.forEach { (locale, label) ->
                    val interaction = remember { MutableInteractionSource() }
                    val hovered by interaction.collectIsHoveredAsState()
                    var focused by remember { mutableStateOf(false) }
                    Row(
                        Modifier.fillMaxWidth()
                            .then(
                                webControlModifier(
                                    "menuitemradio",
                                    selected = app.env.locale == locale,
                                )
                            )
                            .background(
                                if (hovered || focused) MaterialTheme.colorScheme.surfaceVariant
                                else androidx.compose.ui.graphics.Color.Transparent,
                                RoundedCornerShape(6.dp),
                            )
                            .onFocusChanged { focused = it.isFocused }
                            .hoverable(interaction)
                            .clickable(interactionSource = interaction, indication = null) {
                                app.platform.action("locale:$locale")
                                open = false
                            }
                            .padding(start = 6.dp, end = 8.dp, top = 4.dp, bottom = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        Text(label, Modifier.weight(1f))
                        Box(Modifier.size(16.dp), contentAlignment = Alignment.Center) {
                            if (app.env.locale == locale)
                                Icon(
                                    "check",
                                    modifier = Modifier.size(16.dp),
                                    color = MaterialTheme.colorScheme.onBackground,
                                )
                        }
                    }
                }
            }
        }
}
