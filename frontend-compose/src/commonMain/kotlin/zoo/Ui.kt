package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.*
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.*
import androidx.compose.ui.text.font.*
import androidx.compose.ui.text.input.*
import androidx.compose.ui.text.style.*
import androidx.compose.ui.unit.*
import kotlinx.coroutines.*
import kotlinx.serialization.json.*
import org.jetbrains.compose.resources.Font
import zoo_compose.generated.resources.*

data class Fonts(
    val latin: FontFamily,
    val cjk: FontFamily,
    val emoji: FontFamily,
    val mono: FontFamily = FontFamily.Monospace,
    val math: FontFamily = FontFamily.Serif,
    val mathItalic: FontFamily = FontFamily.Serif,
)

val LocalFonts = staticCompositionLocalOf {
    Fonts(FontFamily.Default, FontFamily.Default, FontFamily.Default)
}
val LocalLinkColor = staticCompositionLocalOf { Color(0xff1d4ed8) }
val LocalWebPlatform = staticCompositionLocalOf<Platform?> { null }
val LocalApp = staticCompositionLocalOf<Session> { error("Session is missing") }

@Composable
fun ZooTheme(dark: Boolean, content: @Composable () -> Unit) {
    val latin =
        FontFamily(
            Font(Res.font.Geist),
            Font(Res.font.Geist, FontWeight.Medium),
            Font(Res.font.Geist, FontWeight.SemiBold),
        )
    val cjk =
        FontFamily(
            Font(Res.font.NotoSansJP),
            Font(Res.font.NotoSansJP, FontWeight.Medium),
            Font(Res.font.NotoSansJP, FontWeight.SemiBold),
        )
    val emoji = FontFamily(Font(Res.font.NotoColorEmoji))
    val scheme =
        if (dark)
            darkColorScheme(
                primary = Color(0xffffd600),
                onPrimary = Color.Black,
                background = Color.Black,
                onBackground = Color.White,
                surface = Color(0xff181818),
                onSurface = Color.White,
                surfaceVariant = Color(0xff262626),
                onSurfaceVariant = Color(0xffbdbdbd),
                outline = Color(0xff454545),
                outlineVariant = Color(0xff454545),
                secondary = Color(0xffd9d9d9),
                secondaryContainer = Color(0xff303030),
                onSecondaryContainer = Color(0xffe5e5e5),
            )
        else
            lightColorScheme(
                primary = Color(0xffffd600),
                onPrimary = Color.Black,
                background = Color.White,
                onBackground = Color.Black,
                surface = Color.White,
                onSurface = Color.Black,
                surfaceVariant = Color(0xfff5f5f5),
                onSurfaceVariant = Color(0xff666666),
                outline = Color(0xffe5e5e5),
                outlineVariant = Color(0xffe5e5e5),
                secondary = Color(0xff626262),
                secondaryContainer = Color(0xfffff6cc),
                onSecondaryContainer = Color.Black,
            )
    val mono = FontFamily(Font(Res.font.LiberationMono))
    val math = FontFamily(Font(Res.font.KaTeXMain))
    val mathItalic = FontFamily(Font(Res.font.KaTeXMath))
    val body =
        TextStyle(
            fontFamily = cjk,
            fontSize = 14.sp,
            lineHeight = 20.sp,
            lineHeightStyle =
                LineHeightStyle(LineHeightStyle.Alignment.Center, LineHeightStyle.Trim.None),
        )
    CompositionLocalProvider(
        LocalFonts provides Fonts(latin, cjk, emoji, mono, math, mathItalic),
        LocalLinkColor provides if (dark) Color(0xff8ab4ff) else Color(0xff1d4ed8),
    ) {
        MaterialTheme(
            colorScheme = scheme,
            typography =
                Typography(
                    bodyMedium = body,
                    bodySmall = body.copy(fontSize = 12.sp, lineHeight = 16.sp),
                    titleSmall = body.copy(fontWeight = FontWeight.SemiBold),
                    titleLarge =
                        body.copy(
                            fontSize = 20.sp,
                            lineHeight = 28.sp,
                            fontWeight = FontWeight.SemiBold,
                        ),
                    labelLarge = body,
                    labelSmall = body.copy(fontSize = 11.sp),
                ),
        ) {
            content()
        }
    }
}

fun fontText(text: String, fonts: Fonts): AnnotatedString = buildAnnotatedString {
    append(text)
    var index = 0
    while (index < text.length) {
        val start = index
        val c = text[index]
        val category =
            if (c.isHighSurrogate() || c.code in 0x2600..0x27ff) 2
            else if (c.code <= 0x024f) 0 else 1
        index++
        if (c.isHighSurrogate() && index < text.length && text[index].isLowSurrogate()) index++
        while (index < text.length && text[index].code in listOf(0xfe0f, 0x200d)) {
            index++
            if (index < text.length && text[index].isHighSurrogate()) {
                index++
                if (index < text.length && text[index].isLowSurrogate()) index++
            }
        }
        addStyle(
            SpanStyle(
                fontFamily =
                    when (category) {
                        0 -> fonts.latin
                        2 -> fonts.emoji
                        else -> fonts.cjk
                    }
            ),
            start,
            index,
        )
    }
}

@Composable
fun Text(
    text: String,
    modifier: Modifier = Modifier,
    muted: Boolean = false,
    small: Boolean = false,
    weight: FontWeight? = null,
    color: Color = Color.Unspecified,
    maxLines: Int = Int.MAX_VALUE,
    size: TextUnit = TextUnit.Unspecified,
    onTextLayout: (androidx.compose.ui.text.TextLayoutResult) -> Unit = {},
) {
    val fonts = LocalFonts.current
    androidx.compose.material3.Text(
        remember(text, fonts) { fontText(text, fonts) },
        modifier,
        color =
            if (color != Color.Unspecified) color
            else if (muted) MaterialTheme.colorScheme.onSurfaceVariant
            else
                LocalMfmStyle.current.color.takeIf { it != Color.Unspecified }
                    ?: MaterialTheme.colorScheme.onBackground,
        style =
            (if (small) MaterialTheme.typography.bodySmall else MaterialTheme.typography.bodyMedium)
                .merge(LocalMfmStyle.current),
        fontWeight = weight,
        maxLines = maxLines,
        overflow = TextOverflow.Ellipsis,
        fontSize = size,
        onTextLayout = onTextLayout,
    )
}

private var nextControl = 0

@Composable
fun webControlModifier(role: String, enabled: Boolean = true, selected: Boolean? = null): Modifier {
    val platform = LocalWebPlatform.current
    val id = remember(platform) { "zoo-control-${platform?.uuid() ?: (++nextControl).toString()}" }
    SideEffect { platform?.controlSemantics(id, role, enabled, selected) }
    DisposableEffect(id, platform) { onDispose { platform?.forgetControlSemantics(id) } }
    return Modifier.semantics {
        testTag = id
        if (!enabled) disabled()
        if (selected != null) this.selected = selected
    }
}

@Composable
fun Button(
    label: String,
    enabled: Boolean = true,
    primary: Boolean = false,
    destructive: Boolean = false,
    modifier: Modifier = Modifier,
    role: Role = Role.Button,
    selected: Boolean? = null,
    click: () -> Unit,
) {
    val colors = MaterialTheme.colorScheme
    val background =
        if (destructive) colors.error else if (primary) colors.primary else Color.Transparent
    Box(
        modifier
            .then(webControlModifier(if (role == Role.Tab) "tab" else "button", enabled, selected))
            .defaultMinSize(minHeight = 32.dp)
            .clip(RoundedCornerShape(8.dp))
            .background(background)
            .clickable(enabled, role = role, onClick = { if (enabled) click() })
            .semantics {
                contentDescription = label
                if (!enabled) disabled()
            }
            .padding(horizontal = 12.dp, vertical = 6.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            label,
            color =
                (if (destructive) colors.onError
                    else if (primary) colors.onPrimary else colors.onBackground)
                    .copy(alpha = if (enabled) 1f else .4f),
        )
    }
}

@Composable
fun Icon(
    name: String,
    label: String? = null,
    modifier: Modifier = Modifier,
    color: Color = MaterialTheme.colorScheme.onSurfaceVariant,
) {
    val paths =
        remember(name) {
            (iconPaths[name] ?: iconPaths.getValue("hash")).map {
                PathParser().parsePathString(it).toPath()
            }
        }
    Canvas(
        modifier
            .size(20.dp)
            .then(
                if (label == null) Modifier else Modifier.semantics { contentDescription = label }
            )
    ) {
        withTransform({ scale(size.width / 24f, size.height / 24f, Offset.Zero) }) {
            paths.forEach {
                drawPath(
                    it,
                    color,
                    style = Stroke(2f, cap = StrokeCap.Round, join = StrokeJoin.Round),
                )
            }
        }
    }
}

@Composable
fun IconButton(
    name: String,
    label: String,
    enabled: Boolean = true,
    selected: Boolean = false,
    size: Dp = 32.dp,
    click: () -> Unit,
) {
    Box(
        Modifier.size(size)
            .then(webControlModifier("button", enabled))
            .clip(RoundedCornerShape(8.dp))
            .background(
                if (selected) MaterialTheme.colorScheme.secondaryContainer else Color.Transparent
            )
            .clickable(enabled, role = Role.Button, onClick = { if (enabled) click() })
            .semantics {
                contentDescription = label
                if (!enabled) disabled()
            },
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            name,
            modifier = Modifier.size(if (size >= 40.dp) 20.dp else 16.dp),
            color =
                MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = if (enabled) 1f else .4f),
        )
    }
}

@Composable
fun Field(
    value: String,
    label: String,
    modifier: Modifier = Modifier,
    singleLine: Boolean = true,
    secret: Boolean = false,
    readOnly: Boolean = false,
    onComposition: (Boolean) -> Unit = {},
    chrome: Boolean = true,
    selection: Int? = null,
    onSelection: (Int) -> Unit = {},
    change: (String) -> Unit,
) {
    val fonts = LocalFonts.current
    var editing by remember { mutableStateOf(TextFieldValue(value)) }
    LaunchedEffect(value, selection) {
        if (editing.text != value || selection != null && editing.selection.start != selection)
            editing =
                TextFieldValue(
                    value,
                    TextRange((selection ?: value.length).coerceIn(0, value.length)),
                )
    }
    Column(modifier, verticalArrangement = Arrangement.spacedBy(4.dp)) {
        if (chrome) Text(label, small = true)
        BasicTextField(
            editing,
            {
                editing = it
                onComposition(it.composition != null)
                onSelection(if (it.selection.collapsed) it.selection.end else -1)
                change(it.text)
            },
            modifier =
                Modifier.fillMaxWidth()
                    .heightIn(min = 36.dp, max = 240.dp)
                    .then(
                        if (chrome)
                            Modifier.border(
                                1.dp,
                                MaterialTheme.colorScheme.outline,
                                RoundedCornerShape(8.dp),
                            )
                        else Modifier
                    )
                    .padding(
                        horizontal = if (chrome) 10.dp else 8.dp,
                        vertical = if (chrome) 8.dp else 13.dp,
                    )
                    .semantics { contentDescription = label },
            singleLine = singleLine,
            readOnly = readOnly,
            decorationBox = { inner ->
                Box {
                    if (!chrome && value.isEmpty()) Text(label, muted = true)
                    inner()
                }
            },
            textStyle =
                MaterialTheme.typography.bodyMedium.copy(
                    color = MaterialTheme.colorScheme.onBackground
                ),
            visualTransformation =
                if (secret) PasswordVisualTransformation()
                else
                    VisualTransformation {
                        TransformedText(fontText(it.text, fonts), OffsetMapping.Identity)
                    },
        )
    }
}

@Composable
fun Check(label: String, checked: Boolean, enabled: Boolean = true, change: (Boolean) -> Unit) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.clickable(enabled) { change(!checked) },
    ) {
        Checkbox(checked, change, enabled = enabled, modifier = Modifier.size(32.dp))
        Text(label)
    }
}

@Composable
fun Panel(
    title: String,
    close: () -> Unit,
    width: Dp = 512.dp,
    content: @Composable ColumnScope.() -> Unit,
) {
    OverlayRegistration(title, close, width, body = content)
}

@Composable
fun Confirm(
    title: String,
    description: String,
    action: String,
    busy: Boolean,
    cancel: () -> Unit,
    confirm: () -> Unit,
) {
    val app = LocalApp.current
    Panel(title, { if (!busy) cancel() }) {
        Text(description, muted = true)
        Row(Modifier.align(Alignment.End)) {
            Button(app.t("common.cancelShort"), !busy, click = cancel)
            Button(action, !busy, destructive = true, click = confirm)
        }
    }
}

@Composable
fun Credential(label: String, value: String, secret: Boolean = true) {
    val app = LocalApp.current
    var visible by remember(value) { mutableStateOf(false) }
    Row(verticalAlignment = Alignment.CenterVertically) {
        Field(value, label, Modifier.weight(1f), secret = secret && !visible, readOnly = true) {}
        if (secret)
            IconButton(
                if (visible) "eye-off" else "eye",
                app.t(if (visible) "webhooks.hideSecret" else "webhooks.showSecret"),
            ) {
                visible = !visible
            }
        IconButton("copy", app.t("webhooks.copyLabel", "label" to label)) {
            app.run { app.platform.invoke("copy", value) }
        }
    }
}

data class MenuAction(
    val icon: String,
    val label: String,
    val enabled: Boolean = true,
    val action: () -> Unit,
)

@Composable
fun Menu(label: String, actions: List<MenuAction>, icon: String = "ellipsis", size: Dp = 28.dp) {
    var open by remember { mutableStateOf(false) }
    var anchor by remember { mutableStateOf(androidx.compose.ui.geometry.Rect.Zero) }
    Box(Modifier.onGloballyPositioned { anchor = it.boundsInRoot() }) {
        IconButton(icon, label, size = size) { open = true }
    }
    if (open)
        OverlayRegistration(label, { open = false }, 224.dp, anchor) {
            actions.forEach { item ->
                Row(
                    Modifier.fillMaxWidth()
                        .clickable(item.enabled, role = Role.Button) {
                            open = false
                            item.action()
                        }
                        .padding(horizontal = 8.dp, vertical = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    if (item.icon.isNotBlank()) Icon(item.icon, modifier = Modifier.size(16.dp))
                    Text(item.label, Modifier.padding(start = 8.dp))
                }
            }
        }
}

@Composable
fun ApiContent(
    path: String,
    revision: Int = 0,
    content: @Composable (JsonElement, () -> Unit) -> Unit,
) {
    val app = LocalApp.current
    var data by remember(path) { mutableStateOf<JsonElement?>(null) }
    var failure by remember(path) { mutableStateOf(false) }
    var retry by remember(path) { mutableStateOf(0) }
    LaunchedEffect(path, revision, retry) {
        try {
            data = app.api.request(path)
            failure = false
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            failure = true
        }
    }
    if (failure) {
        Text(app.t("common.errors.network"))
        Button(app.t("offline.retry")) { retry++ }
    } else data?.let { content(it) { retry++ } } ?: Text(app.t("common.loading"), muted = true)
}

fun JsonElement.rows(): List<Row> =
    when (this) {
        is JsonArray -> mapNotNull { it as? JsonObject }
        is JsonObject ->
            if ("txid" in this)
                get("data")
                    ?.let { (it as? JsonArray)?.mapNotNull { row -> row as? JsonObject } }
                    .orEmpty()
            else emptyList()
        else -> emptyList()
    }

fun obj(vararg values: Pair<String, String?>) = buildJsonObject {
    values.forEach { (key, value) -> put(key, value?.let(::JsonPrimitive) ?: JsonNull) }
}
