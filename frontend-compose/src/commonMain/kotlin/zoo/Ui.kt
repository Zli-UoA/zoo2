package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.interaction.collectIsFocusedAsState
import androidx.compose.foundation.interaction.collectIsHoveredAsState
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.*
import androidx.compose.ui.graphics.drawscope.*
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.input.key.*
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalWindowInfo
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
val LocalViewportWidth = staticCompositionLocalOf { Float.POSITIVE_INFINITY }
val LocalApp = staticCompositionLocalOf<Session> { error("Session is missing") }

internal expect fun webTextStyle(): PlatformTextStyle

@Composable
fun ZooTheme(
    dark: Boolean,
    viewportWidth: Float? = null,
    locale: String = "ja",
    content: @Composable () -> Unit,
) {
    val latin =
        FontFamily(
            Font(Res.font.Geist),
            Font(Res.font.Geist, FontWeight.Medium),
            Font(Res.font.Geist, FontWeight.SemiBold),
        )
    val cjk =
        FontFamily(
            Font(Res.font.NotoSansJPRegular),
            Font(
                Res.font.NotoSansJPRegular,
                FontWeight.Medium,
                variationSettings = FontVariation.Settings(FontWeight.Normal, FontStyle.Normal),
            ),
            Font(
                Res.font.NotoSansJPBold,
                FontWeight.SemiBold,
                variationSettings = FontVariation.Settings(FontWeight.Bold, FontStyle.Normal),
            ),
        )
    val emoji = FontFamily(Font(Res.font.NotoColorEmoji))
    val scheme =
        if (dark)
            darkColorScheme(
                error = Color(0xffff6467),
                onError = Color.Black,
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
                error = Color(0xffc2000f),
                onError = Color.White,
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
            fontFamily = latin,
            localeList = androidx.compose.ui.text.intl.LocaleList(locale),
            fontSize = 14.sp,
            lineHeight = browserLineHeight(20.sp),
            platformStyle = webTextStyle(),
            lineHeightStyle =
                LineHeightStyle(LineHeightStyle.Alignment.Center, LineHeightStyle.Trim.None),
        )
    CompositionLocalProvider(
        LocalViewportWidth provides
            (viewportWidth
                ?: with(LocalDensity.current) {
                    LocalWindowInfo.current.containerSize.width.toDp().value
                }),
        LocalFonts provides Fonts(latin, cjk, emoji, mono, math, mathItalic),
        LocalLinkColor provides if (dark) Color(0xff8ab4ff) else Color(0xff1d4ed8),
    ) {
        MaterialTheme(
            colorScheme = scheme,
            typography =
                Typography(
                    bodyMedium = body,
                    bodySmall = body.copy(fontSize = 12.sp, lineHeight = browserLineHeight(16.sp)),
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

fun fontText(text: String, fonts: Fonts, weight: FontWeight? = null): AnnotatedString =
    buildAnnotatedString {
        append(text)
        fun run(category: Int, start: Int, end: Int) {
            addStyle(
                SpanStyle(
                    fontFamily =
                        when (category) {
                            0 -> fonts.latin
                            2 -> fonts.emoji
                            else -> fonts.cjk
                        },
                    fontWeight = weight,
                ),
                start,
                end,
            )
        }
        var index = 0
        var start = 0
        var previous = -1
        fun point(at: Int): Int {
            val first = text[at]
            return if (
                first.isHighSurrogate() && at + 1 < text.length && text[at + 1].isLowSurrogate()
            )
                0x10000 + ((first.code - 0xd800) shl 10) + text[at + 1].code - 0xdc00
            else first.code
        }
        fun next(at: Int): Int = at + if (point(at) > 0xffff) 2 else 1
        while (index < text.length) {
            val base = point(index)
            var end = next(index)
            var emojiCluster = false
            var textPresentation = false
            if (base in 0x1f1e6..0x1f1ff && end < text.length && point(end) in 0x1f1e6..0x1f1ff) {
                end = next(end)
                emojiCluster = true
            }
            while (end < text.length) {
                val code = point(end)
                when {
                    code == 0xfe0f -> {
                        emojiCluster = true
                        end = next(end)
                    }
                    code == 0xfe0e -> {
                        textPresentation = true
                        end = next(end)
                    }
                    code == 0x200d -> {
                        emojiCluster = true
                        end = next(end)
                        if (end < text.length) end = next(end)
                    }
                    code in 0x1f3fb..0x1f3ff || code == 0x20e3 || code in 0xe0020..0xe007f -> {
                        emojiCluster = true
                        end = next(end)
                    }
                    code <= 0xffff &&
                        text[end].category in
                            listOf(
                                CharCategory.NON_SPACING_MARK,
                                CharCategory.COMBINING_SPACING_MARK,
                                CharCategory.ENCLOSING_MARK,
                            ) -> end = next(end)
                    else -> break
                }
            }
            var allLatin = true
            var cursor = index
            while (cursor < end) {
                val code = point(cursor)
                if (code !in listOf(0xfe0e, 0xfe0f) && !CssFontCoverage.latinContains(code))
                    allLatin = false
                cursor = next(cursor)
            }
            val category =
                when {
                    emojiCluster && !textPresentation -> 2
                    allLatin -> 0
                    !textPresentation && CssFontCoverage.emojiContains(base) -> 2
                    else -> 1
                }
            if (category != previous) {
                if (previous >= 0) run(previous, start, index)
                start = index
                previous = category
            }
            index = end
        }
        if (previous >= 0) run(previous, start, text.length)
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
    lineHeight: TextUnit = TextUnit.Unspecified,
    textAlign: TextAlign? = null,
    letterSpacing: TextUnit = TextUnit.Unspecified,
    onTextLayout: (androidx.compose.ui.text.TextLayoutResult) -> Unit = {},
) {
    val fonts = LocalFonts.current
    androidx.compose.material3.Text(
        remember(text, fonts, weight) { fontText(text, fonts, weight) },
        modifier,
        color =
            if (color != Color.Unspecified) color
            else if (muted) MaterialTheme.colorScheme.onSurfaceVariant
            else
                LocalMfmStyle.current.color.takeIf { it != Color.Unspecified }
                    ?: MaterialTheme.colorScheme.onBackground,
        style =
            (if (small) MaterialTheme.typography.bodySmall else MaterialTheme.typography.bodyMedium)
                .merge(LocalMfmStyle.current)
                .let { it.copy(lineHeight = browserLineHeight(it.lineHeight)) },
        fontWeight = weight,
        maxLines = maxLines,
        overflow = TextOverflow.Ellipsis,
        fontSize = size,
        lineHeight = browserLineHeight(lineHeight),
        textAlign = textAlign,
        letterSpacing = letterSpacing,
        onTextLayout = onTextLayout,
    )
}

/** Avoid a rounded-up extra pixel when SkParagraph reconstructs an integral CSS line height. */
internal fun browserLineHeight(height: TextUnit): TextUnit =
    if (height.isSp && height.value > 0f && height.value % 1f == 0f)
        Float.fromBits(height.value.toBits() - 1).sp
    else height

private var nextControl = 0

@Composable
fun webControlModifier(
    role: String,
    enabled: Boolean = true,
    selected: Boolean? = null,
    expanded: Boolean? = null,
    busy: Boolean? = null,
    attributes: Map<String, String> = emptyMap(),
): Modifier {
    val platform = LocalWebPlatform.current
    val id = remember(platform) { "zoo-control-${platform?.uuid() ?: (++nextControl).toString()}" }
    val interactive =
        role in
            setOf(
                "button",
                "tab",
                "menuitem",
                "menuitemradio",
                "menuitemcheckbox",
                "radio",
                "checkbox",
                "switch",
                "link",
                "combobox",
                "textbox",
                "searchbox",
                "option",
                "spinbutton",
                "slider",
                "treeitem",
            )
    val requester =
        remember(interactive) {
            if (interactive) androidx.compose.ui.focus.FocusRequester() else null
        }
    SideEffect {
        platform?.controlSemantics(
            id,
            role,
            enabled,
            selected,
            expanded,
            attributes + if (busy == null) emptyMap() else mapOf("aria-busy" to busy.toString()),
        )
        if (requester != null) platform?.controlFocus(id) { if (enabled) requester.requestFocus() }
    }
    DisposableEffect(id, platform) { onDispose { platform?.forgetControlSemantics(id) } }
    return (if (requester == null) Modifier else Modifier.focusRequester(requester)).semantics {
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
    expanded: Boolean? = null,
    variant: String = if (primary) "default" else if (destructive) "destructive" else "ghost",
    size: String = "default",
    icon: String? = null,
    inlineStartIcon: Boolean = false,
    iconSize: Dp? = null,
    contentGap: Dp? = null,
    textStyle: TextStyle? = null,
    title: String? = null,
    accessibleLabel: String? = null,
    click: () -> Unit,
) {
    val activation = LocalControlActivation.current
    val colors = MaterialTheme.colorScheme
    val dark = colors.background == Color.Black
    val interaction = remember {
        androidx.compose.foundation.interaction.MutableInteractionSource()
    }
    val hovered by interaction.collectIsHoveredAsState()
    val focused by interaction.collectIsFocusedAsState()
    val focusVisible = enabled && focused && FocusInputModality.keyboard
    val isHovered = enabled && hovered
    val density = LocalDensity.current
    var labelAdvance by remember(label, size, textStyle) { mutableStateOf(Float.NaN) }
    val controlHeight =
        if (variant == "tab") 36.dp
        else
            when (size.removePrefix("icon-")) {
                "xs" -> 24.dp
                "sm" -> 28.dp
                "lg" -> 36.dp
                else -> 32.dp
            }
    val iconOnly = size.startsWith("icon")
    val radius =
        if (variant in listOf("toggle", "tab") || size.endsWith("xs") || size.endsWith("sm")) 8.dp
        else 10.dp
    val horizontalPadding =
        if (variant == "tab") 12.dp else if (iconOnly) 0.dp else if (size == "xs") 9.dp else 11.dp
    val startPadding =
        if (inlineStartIcon && icon != null && !iconOnly) horizontalPadding - 2.dp
        else horizontalPadding
    val endPadding = horizontalPadding
    val resolvedIconSize =
        iconSize
            ?: when (size) {
                "xs",
                "icon-xs" -> 12.dp
                "sm" -> 14.dp
                else -> 16.dp
            }
    val gap = contentGap ?: if (size == "xs" || size == "sm") 4.dp else 6.dp
    val preferredWidth =
        with(density) {
            if (iconOnly) controlHeight.toPx()
            else
                labelAdvance +
                    startPadding.toPx() +
                    endPadding.toPx() +
                    if (icon != null) resolvedIconSize.toPx() + gap.toPx() else 0f
        }
    val shape = RoundedCornerShape(radius)
    val destructiveColor = if (dark) Color(0xffff6467) else Color(0xffc2000f)
    val focusBorder =
        if (variant == "destructive") destructiveColor.copy(alpha = .4f)
        else if (dark) Color(0xffbdbdbd) else Color(0xff737373)
    val focusRing =
        if (variant == "destructive") destructiveColor.copy(alpha = .2f)
        else focusBorder.copy(alpha = .5f)
    val textColor =
        textStyle?.color?.takeIf { it != Color.Unspecified }
            ?: when (variant) {
                "destructive" -> destructiveColor
                "link" -> LocalLinkColor.current
                "default" -> if (enabled) colors.onPrimary else colors.onSurfaceVariant
                "tab" ->
                    if (selected == true || isHovered) colors.onBackground
                    else colors.onSurfaceVariant
                "toggle" -> if (selected == true) colors.onPrimary else colors.onBackground
                else -> colors.onBackground
            }
    val background =
        when (variant) {
            "default" ->
                if (!enabled) colors.surfaceVariant
                else if (isHovered) Color(0xffffe34d) else colors.primary
            "outline" ->
                if (dark && isHovered) Color(0xff575757).copy(alpha = .5f)
                else if (expanded == true) colors.surfaceVariant
                else if (dark) Color(0xff575757).copy(alpha = .3f)
                else if (isHovered) colors.surfaceVariant else colors.background
            "tab" -> if (selected == true) colors.background else Color.Transparent
            "secondary" ->
                if (isHovered) {
                    if (dark) Color(0xff2f2f2f) else Color(0xffe5e5e5)
                } else colors.surfaceVariant
            "toggle" ->
                if (selected == true) colors.primary
                else if (isHovered) colors.surfaceVariant else Color.Transparent
            "destructive" ->
                destructiveColor.copy(
                    alpha =
                        if (dark) {
                            if (isHovered) .3f else .2f
                        } else {
                            if (isHovered) .2f else .1f
                        }
                )
            else ->
                if (variant == "ghost" && dark && isHovered) colors.surfaceVariant.copy(alpha = .5f)
                else if ((isHovered || expanded == true) && variant == "ghost")
                    colors.surfaceVariant
                else Color.Transparent
        }
    Box(
        modifier
            .cssPreferredWidth(preferredWidth)
            .graphicsLayer { alpha = if (!enabled && variant != "default") .5f else 1f }
            .then(
                webControlModifier(
                    if (role == Role.Tab) "tab" else "button",
                    enabled,
                    selected,
                    expanded = expanded,
                    attributes = if (title == null) emptyMap() else mapOf("title" to title),
                )
            )
            .height(controlHeight)
            .then(if (iconOnly) Modifier.width(controlHeight) else Modifier)
            .then(
                if (variant == "tab" && selected == true) Modifier.cssShadowSm(shape) else Modifier
            )
            .focusInputModality()
            .focusVisibleRing(radius, focusRing, focusVisible, preferredWidth = preferredWidth)
            .clip(shape)
            .drawBehind {
                val width = cssPaintWidth(this.size.width, preferredWidth)
                val border = if (variant == "tab") 0f else 1.dp.toPx()
                val corner = radius.toPx()
                // React's bg-clip-padding excludes the transparent one-pixel border.
                drawRoundRect(
                    background,
                    Offset(border, border),
                    androidx.compose.ui.geometry.Size(
                        (width - 2 * border).coerceAtLeast(0f),
                        (this.size.height - 2 * border).coerceAtLeast(0f),
                    ),
                    androidx.compose.ui.geometry.CornerRadius((corner - border).coerceAtLeast(0f)),
                )
                val borderColor =
                    if (focusVisible) focusBorder
                    else if (variant == "outline") {
                        if (dark) Color(0xff575757) else colors.outline
                    } else Color.Transparent
                if (variant != "tab")
                    drawRoundRect(
                        borderColor,
                        Offset(border / 2, border / 2),
                        androidx.compose.ui.geometry.Size(
                            width - border,
                            this.size.height - border,
                        ),
                        androidx.compose.ui.geometry.CornerRadius(corner - border / 2),
                        style = Stroke(border),
                    )
            }
            .hoverable(interaction, enabled)
            .clickable(
                interactionSource = interaction,
                indication = null,
                enabled = enabled,
                role = role,
                onClick = {
                    if (enabled) {
                        activation?.invoke()
                        click()
                    }
                },
            )
            .semantics {
                contentDescription = accessibleLabel ?: label
                if (!enabled) disabled()
            }
            .padding(start = startPadding, end = endPadding),
        contentAlignment = Alignment.Center,
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(gap),
        ) {
            if (icon != null)
                Icon(icon, modifier = Modifier.size(resolvedIconSize), color = textColor)
            if (!iconOnly)
                CompositionLocalProvider(
                    LocalMfmStyle provides
                        LocalMfmStyle.current.merge(textStyle ?: TextStyle.Default)
                ) {
                    Text(
                        label,
                        weight = textStyle?.fontWeight ?: FontWeight.Medium,
                        lineHeight = textStyle?.lineHeight ?: TextUnit.Unspecified,
                        size =
                            textStyle?.fontSize?.takeIf { it.isSpecified }
                                ?: when (size) {
                                    "xs" -> 12.sp
                                    "sm" -> 12.8.sp
                                    else -> 14.sp
                                },
                        color = textColor,
                        maxLines = 1,
                        modifier =
                            Modifier.drawBehind {
                                if (variant == "link" && isHovered) {
                                    val y = this.size.height - 2.dp.toPx()
                                    drawLine(
                                        textColor,
                                        Offset(0f, y),
                                        Offset(this.size.width, y),
                                        strokeWidth = 1.dp.toPx(),
                                    )
                                }
                            },
                        onTextLayout = { layout ->
                            if (layout.lineCount > 0)
                                labelAdvance = layout.getLineRight(0) - layout.getLineLeft(0)
                        },
                    )
                }
        }
    }
}

@Composable
fun Icon(
    name: String,
    label: String? = null,
    modifier: Modifier = Modifier,
    color: Color = MaterialTheme.colorScheme.onSurfaceVariant,
) {
    var pixelSize by remember { mutableStateOf(IntSize.Zero) }
    var pixelOrigin by remember { mutableStateOf(Offset.Zero) }
    val bitmap = rememberIconRaster(name, color, pixelSize)
    val paths =
        remember(name) {
            (iconPaths[name] ?: iconPaths.getValue("hash")).map {
                PathParser().parsePathString(it).toPath()
            }
        }
    Canvas(
        modifier
            .size(20.dp)
            .onSizeChanged { pixelSize = it }
            .onGloballyPositioned { pixelOrigin = it.positionInRoot() }
            .then(
                if (label == null) Modifier
                else
                    Modifier.semantics {
                        role = Role.Image
                        contentDescription = label
                    }
            )
    ) {
        // Chrome snaps an SVG viewport's origin to device pixels, while its
        // layout and hit bounds retain fractional CSS coordinates.
        withTransform({
            translate(
                kotlin.math.floor(pixelOrigin.x + .5f) - pixelOrigin.x,
                kotlin.math.floor(pixelOrigin.y + .5f) - pixelOrigin.y,
            )
        }) {
            if (bitmap != null) {
                drawImage(bitmap, dstSize = IntSize(size.width.toInt(), size.height.toInt()))
            } else
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
}

@Composable
fun IconButton(
    name: String,
    label: String,
    enabled: Boolean = true,
    selected: Boolean = false,
    size: Dp = 32.dp,
    modifier: Modifier = Modifier,
    color: Color = MaterialTheme.colorScheme.onSurfaceVariant,
    click: () -> Unit,
) {
    Box(
        modifier
            .size(size)
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
            color = color.copy(alpha = color.alpha * if (enabled) 1f else .4f),
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
    fileOwner: String? = null,
    showLabel: Boolean = chrome,
    enabled: Boolean = true,
    placeholder: String? = null,
    invalid: Boolean = false,
    inputRole: String? = null,
    role: String? = inputRole,
    inputTextStyle: TextStyle? = null,
    onBlur: () -> Unit = {},
    leadingIcon: String? = null,
    autoFocus: Boolean = false,
    focusRequester: androidx.compose.ui.focus.FocusRequester? = null,
    minimumHeight: Dp? = null,
    contentPadding: PaddingValues? = null,
    resizable: Boolean = chrome && !singleLine,
    change: (String) -> Unit,
) {
    val density = LocalDensity.current
    var resizedHeight by remember { mutableStateOf<Dp?>(null) }
    var fieldSize by remember { mutableStateOf(IntSize.Zero) }
    val fonts = LocalFonts.current
    val webPlatform = LocalWebPlatform.current
    val defaultRequester = remember { androidx.compose.ui.focus.FocusRequester() }
    val requester = focusRequester ?: defaultRequester
    var focused by remember { mutableStateOf(false) }
    val blur by rememberUpdatedState(onBlur)
    var editing by remember { mutableStateOf(TextFieldValue(value)) }
    val layout = remember { arrayOfNulls<TextLayoutResult>(1) }
    val viewport = LocalViewportWidth.current
    val colors = MaterialTheme.colorScheme
    val dark = colors.background == Color.Black
    val inputColor = if (dark) Color(0xff575757) else Color(0xffd4d4d4)
    val errorColor = if (dark) Color(0xffff6467) else Color(0xffc2000f)
    val ringColor =
        if (invalid && !singleLine) errorColor.copy(alpha = if (dark) .4f else .2f)
        else if (dark) Color(0xffbdbdbd).copy(alpha = .5f) else Color(0xff737373).copy(alpha = .5f)
    val borderColor =
        if (invalid && !singleLine) errorColor.copy(alpha = if (dark) .5f else 1f)
        else if (focused) if (dark) Color(0xffbdbdbd) else Color(0xff737373)
        else if (singleLine) colors.outline else inputColor
    val shape = RoundedCornerShape(10.dp)
    val type =
        MaterialTheme.typography.bodyMedium
            .copy(
                fontSize = if ((!chrome || !singleLine) && viewport < 768f) 16.sp else 14.sp,
                lineHeight = if (!chrome || !singleLine && viewport < 768f) 24.sp else 20.sp,
                color = colors.onBackground,
            )
            .merge(inputTextStyle)
            .let {
                if (inputTextStyle?.fontFamily == FontFamily.Monospace)
                    it.copy(fontFamily = fonts.mono)
                else it
            }
            .let { it.copy(lineHeight = browserLineHeight(it.lineHeight)) }
    val fieldFonts =
        fonts.copy(
            latin =
                inputTextStyle?.fontFamily?.let {
                    if (it == FontFamily.Monospace) fonts.mono else it
                } ?: fonts.latin
        )
    LaunchedEffect(autoFocus, enabled) { if (autoFocus && enabled) requester.requestFocus() }
    LaunchedEffect(value, selection) {
        if (editing.text != value || selection != null && editing.selection.start != selection)
            editing =
                TextFieldValue(
                    value,
                    TextRange((selection ?: value.length).coerceIn(0, value.length)),
                )
    }
    val attributes =
        buildMap<String, String> {
            if (invalid) put("aria-invalid", "true")
            if (readOnly) put("aria-readonly", "true")
        }
    Column(modifier, verticalArrangement = Arrangement.spacedBy(4.dp)) {
        if (showLabel) Text(label, small = true)
        Box {
            BasicTextField(
                editing,
                {
                    editing = it
                    onComposition(it.composition != null)
                    onSelection(if (it.selection.collapsed) it.selection.end else -1)
                    change(it.text)
                },
                modifier =
                    Modifier.then(
                            if (role != null || !secret)
                                webControlModifier(
                                    role ?: "textbox",
                                    enabled,
                                    attributes = attributes,
                                )
                            else Modifier
                        )
                        .then(Modifier.focusRequester(requester))
                        .onPreviewKeyEvent {
                            if (
                                it.type == KeyEventType.KeyDown &&
                                    it.key == Key.MoveEnd &&
                                    !it.isCtrlPressed &&
                                    !it.isMetaPressed &&
                                    !it.isAltPressed &&
                                    !it.isShiftPressed &&
                                    editing.composition == null
                            ) {
                                val caret = editing.selection.end
                                val end =
                                    layout[0]?.let { result ->
                                        result.getLineEnd(
                                            result.getLineForOffset(caret),
                                            visibleEnd = false,
                                        )
                                    }
                                        ?: editing.text.indexOf('\n', caret).takeIf { it >= 0 }
                                        ?: editing.text.length
                                val position =
                                    if (end > 0 && editing.text.getOrNull(end - 1) == '\n') end - 1
                                    else end
                                editing = editing.copy(selection = TextRange(position))
                                onSelection(position)
                                true
                            } else false
                        }
                        .onFocusChanged {
                            val leaving = focused && !it.isFocused
                            focused = it.isFocused
                            if (it.isFocused)
                                webPlatform?.action("fileFocus:${fileOwner.orEmpty()}")
                            if (leaving) blur()
                        }
                        .fillMaxWidth()
                        .onSizeChanged { fieldSize = it }
                        .then(
                            if (resizable && resizedHeight != null) Modifier.height(resizedHeight!!)
                            else if (chrome && singleLine) Modifier.height(minimumHeight ?: 32.dp)
                            else
                                Modifier.heightIn(
                                    min = minimumHeight ?: if (!chrome) 44.dp else 64.dp,
                                    max = if (chrome) Dp.Infinity else 200.dp,
                                )
                        )
                        .graphicsLayer { alpha = if (!enabled && !singleLine) .5f else 1f }
                        .drawBehind {
                            if (chrome && (focused || invalid && !singleLine))
                                drawRoundRect(
                                    ringColor,
                                    Offset((-1.5).dp.toPx(), (-1.5).dp.toPx()),
                                    androidx.compose.ui.geometry.Size(
                                        size.width + 3.dp.toPx(),
                                        size.height + 3.dp.toPx(),
                                    ),
                                    androidx.compose.ui.geometry.CornerRadius(11.5.dp.toPx()),
                                    style = Stroke(3.dp.toPx()),
                                )
                        }
                        .then(
                            if (chrome)
                                Modifier.background(
                                        if (singleLine) colors.background
                                        else if (dark)
                                            inputColor.copy(alpha = if (enabled) .3f else .8f)
                                        else if (!enabled) inputColor.copy(alpha = .5f)
                                        else Color.Transparent,
                                        shape,
                                    )
                                    .border(1.dp, borderColor, shape)
                            else Modifier
                        )
                        .padding(
                            contentPadding
                                ?: PaddingValues(
                                    start =
                                        if (leadingIcon != null) 32.dp
                                        else if (chrome) if (singleLine) 9.dp else 11.dp else 8.dp,
                                    end = if (chrome) if (singleLine) 9.dp else 11.dp else 8.dp,
                                    top = if (chrome) if (singleLine) 6.dp else 9.dp else 10.dp,
                                    bottom = if (chrome) if (singleLine) 6.dp else 9.dp else 10.dp,
                                )
                        )
                        .semantics { contentDescription = label },
                onTextLayout = { layout[0] = it },
                singleLine = singleLine,
                enabled = enabled,
                readOnly = readOnly,
                cursorBrush = SolidColor(colors.onBackground),
                decorationBox = { inner ->
                    Box {
                        if (leadingIcon != null)
                            Box(Modifier.matchParentSize()) {
                                Icon(
                                    leadingIcon,
                                    modifier =
                                        Modifier.align(Alignment.CenterStart)
                                            .offset(x = (-22).dp)
                                            .size(16.dp),
                                )
                            }
                        val placeholderText = placeholder ?: if (chrome) "" else label
                        if (value.isEmpty() && placeholderText.isNotEmpty())
                            androidx.compose.material3.Text(
                                fontText(placeholderText, fieldFonts),
                                color = colors.onSurfaceVariant,
                                style = type,
                            )
                        inner()
                    }
                },
                textStyle = type,
                visualTransformation =
                    if (secret) PasswordVisualTransformation()
                    else
                        VisualTransformation {
                            TransformedText(fontText(it.text, fieldFonts), OffsetMapping.Identity)
                        },
            )
            if (resizable)
                Canvas(
                    Modifier.align(Alignment.BottomEnd).size(12.dp).pointerInput(
                        enabled,
                        minimumHeight,
                    ) {
                        if (enabled)
                            detectDragGestures { change, amount ->
                                change.consume()
                                with(density) {
                                    resizedHeight =
                                        ((resizedHeight ?: fieldSize.height.toDp()) +
                                                amount.y.toDp())
                                            .coerceAtLeast(minimumHeight ?: 64.dp)
                                }
                            }
                    }
                ) {
                    val grip =
                        (if (dark) Color(0xff999999) else Color(0xff666666)).copy(
                            alpha = if (enabled) 1f else .5f
                        )
                    fun diagonal(length: Int, startY: Int) {
                        repeat(length) { index ->
                            drawRect(
                                grip,
                                Offset((8 - index).dp.toPx(), (startY + index).dp.toPx()),
                                androidx.compose.ui.geometry.Size(1.dp.toPx(), 1.dp.toPx()),
                            )
                        }
                    }
                    diagonal(7, 2)
                    diagonal(3, 6)
                }
        }
    }
}

@Composable
fun Check(label: String, checked: Boolean, enabled: Boolean = true, change: (Boolean) -> Unit) {
    Row(
        Modifier.then(webControlModifier("checkbox", enabled, checked))
            .toggleable(checked, enabled, role = Role.Checkbox, onValueChange = change),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Box(
            Modifier.size(13.dp)
                .border(
                    1.dp,
                    if (checked) Color(0xff0075ff) else Color(0xff767676),
                    RoundedCornerShape(2.dp),
                )
                .background(
                    if (checked) Color(0xff0075ff) else MaterialTheme.colorScheme.background,
                    RoundedCornerShape(2.dp),
                ),
            contentAlignment = Alignment.Center,
        ) {
            if (checked) Icon("check", modifier = Modifier.size(11.dp), color = Color.White)
        }
        Text(label)
    }
}

@Composable
fun Panel(
    title: String,
    close: () -> Unit,
    width: Dp = 384.dp,
    alert: Boolean = false,
    description: String = "",
    footer: (@Composable () -> Unit)? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    OverlayRegistration(
        title,
        close,
        width,
        body = content,
        alert = alert,
        description = description,
        footer = footer,
    )
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
    ConfirmationPanel(title, description, action, busy, null, cancel, confirm)
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
fun Menu(
    label: String,
    actions: List<MenuAction>,
    icon: String = "ellipsis",
    size: Dp = 28.dp,
    controlledOpen: Boolean? = null,
    onOpenChange: (Boolean) -> Unit = {},
) {
    var internalOpen by remember { mutableStateOf(false) }
    val open = controlledOpen ?: internalOpen
    val trigger = remember { androidx.compose.ui.focus.FocusRequester() }
    val requesters =
        remember(actions.map { it.label }) {
            actions.map { androidx.compose.ui.focus.FocusRequester() }
        }
    var active by remember { mutableStateOf(0) }
    fun setOpen(value: Boolean) {
        internalOpen = value
        onOpenChange(value)
        if (!value) trigger.requestFocus()
    }
    var anchor by remember { mutableStateOf(androidx.compose.ui.geometry.Rect.Zero) }
    Box(
        Modifier.focusRequester(trigger)
            .onGloballyPositioned { anchor = it.boundsInRoot() }
            .onPreviewKeyEvent {
                if (
                    it.type == KeyEventType.KeyDown &&
                        (it.key == Key.DirectionDown || it.key == Key.DirectionUp)
                ) {
                    setOpen(true)
                    true
                } else false
            }
    ) {
        IconButton(icon, label, size = size) { setOpen(true) }
    }
    if (open)
        OverlayRegistration(label, { setOpen(false) }, 224.dp, anchor, role = "menu") {
            LaunchedEffect(Unit) {
                withFrameNanos {}
                actions
                    .indexOfFirst { it.enabled }
                    .takeIf { it >= 0 }
                    ?.let { requesters[it].requestFocus() }
            }
            Column(
                Modifier.onPreviewKeyEvent {
                    if (it.type != KeyEventType.KeyDown) false
                    else if (it.key == Key.Escape || it.key == Key.Tab) {
                        setOpen(false)
                        it.key == Key.Escape
                    } else {
                        val enabled = actions.indices.filter { index -> actions[index].enabled }
                        val current = enabled.indexOf(active).coerceAtLeast(0)
                        val target =
                            when (it.key) {
                                Key.DirectionDown ->
                                    enabled.getOrNull((current + 1) % enabled.size.coerceAtLeast(1))
                                Key.DirectionUp ->
                                    enabled.getOrNull(
                                        (current - 1 + enabled.size) % enabled.size.coerceAtLeast(1)
                                    )
                                Key.MoveHome -> enabled.firstOrNull()
                                Key.MoveEnd -> enabled.lastOrNull()
                                else -> null
                            }
                        if (target != null) {
                            requesters[target].requestFocus()
                            true
                        } else false
                    }
                }
            ) {
                actions.forEachIndexed { index, item ->
                    Row(
                        Modifier.fillMaxWidth()
                            .then(webControlModifier("menuitem", item.enabled))
                            .focusRequester(requesters[index])
                            .onFocusChanged { if (it.isFocused) active = index }
                            .clickable(item.enabled) {
                                setOpen(false)
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

fun obj(vararg values: Pair<String, Any?>) = buildJsonObject {
    values.forEach { (key, value) ->
        put(
            key,
            when (value) {
                null -> JsonNull
                is JsonElement -> value
                is String -> JsonPrimitive(value)
                is Boolean -> JsonPrimitive(value)
                is Number -> JsonPrimitive(value)
                else -> error("Unsupported JSON value for $key")
            },
        )
    }
}
