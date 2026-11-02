package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.positionInRoot
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.*
import kotlinx.serialization.json.*

@Composable
fun PrimitiveStory(app: Session, options: Row) {
    val fixture = options.nested("fixture")
    val title = fixture.text("primitive")
    val state = fixture.text("state")
    val dark = options.flag("dark")
    val dropdown = fixture.nested("dropdown")
    val tooltip = fixture.nested("tooltip")
    val toastFixture = fixture.nested("toast")
    when (title) {
        "Components/ScrollArea" ->
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                ScrollArea(
                    Modifier.size(288.dp, 256.dp)
                        .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(10.dp)),
                    contentPadding = 13.dp,
                ) {
                    val rows = fixture.array("scrollRows")
                    if (rows.isEmpty())
                        Text(fixture.text("content"), size = 16.sp, lineHeight = 24.sp)
                    else
                        rows.forEach { row ->
                            Text(
                                row.jsonPrimitive.content,
                                Modifier.padding(vertical = 12.dp),
                                size = 16.sp,
                                lineHeight = 24.sp,
                            )
                            HorizontalDivider()
                        }
                }
            }
        "Components/CollapsibleSection" ->
            Box(
                Modifier.width(288.dp)
                    .background(if (dark) Color(0xff141414) else Color(0xfffafaf8))
            ) {
                val action: (() -> Unit)? =
                    if (fixture.flag("hasActions"))
                        ({
                            app.run {
                                app.platform.invoke(
                                    "storyCallback",
                                    buildJsonObject {
                                            put("name", "actions.props.onClick")
                                            put("args", JsonArray(emptyList()))
                                        }
                                        .toString(),
                                )
                            }
                        })
                    else null
                CollapsibleSection(
                    fixture.text("title"),
                    fixture.text("count").takeIf { it.isNotEmpty() },
                    defaultOpen = fixture.flag("defaultOpen"),
                    action = action,
                ) {
                    Text(
                        fixture.text("content"),
                        Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    )
                }
            }
        "Components/DropdownMenu" ->
            Box(
                Modifier.fillMaxWidth().heightIn(min = 288.dp).padding(48.dp),
                contentAlignment = Alignment.TopCenter,
            ) {
                ActionMenu(
                    if (state == "Icon Only Trigger") dropdown.text("iconTrigger")
                    else dropdown.text("trigger"),
                    initialOpen = state != "Default",
                    iconOnly = state == "Icon Only Trigger",
                    actions =
                        dropdown.array("items").map { node ->
                            val item = node.objectOrEmpty()
                            MenuAction(item.text("icon"), item.text("label")) {}
                        },
                )
            }
        "Components/Tooltip" ->
            Box(
                Modifier.fillMaxWidth().heightIn(min = 288.dp).padding(48.dp),
                contentAlignment = Alignment.Center,
            ) {
                Hint(tooltip.text("content")) { modifier ->
                    Button(tooltip.text("trigger"), variant = "outline", modifier = modifier) {}
                }
            }
        "Components/Toaster" -> {
            Box(Modifier.fillMaxSize()) {
                Button(
                    toastFixture.text("trigger"),
                    primary = true,
                    modifier = Modifier.padding(16.dp),
                ) {
                    app.showToast(
                        toastFixture.text("content"),
                        ToastKind.Success,
                        durationMillis = null,
                    )
                }
            }
        }
        "Foundations/Tokens" -> TokensStory(app, dark)
        else -> error("Unknown primitive story: $title / $state")
    }
}

@Composable
private fun TokensStory(app: Session, dark: Boolean) {
    var brandTop by remember { mutableStateOf(0f) }
    var linkAdvance by remember { mutableStateOf(Float.NaN) }
    var badgeAdvance by remember { mutableStateOf(Float.NaN) }
    val density = androidx.compose.ui.platform.LocalDensity.current
    val bg = MaterialTheme.colorScheme.background
    val fg = MaterialTheme.colorScheme.onBackground
    val muted = MaterialTheme.colorScheme.surfaceVariant
    val mutedFg = MaterialTheme.colorScheme.onSurfaceVariant
    val pairs =
        listOf(
            Triple("background", "foreground", bg to fg),
            Triple("card", "card-foreground", MaterialTheme.colorScheme.surface to fg),
            Triple("popover", "popover-foreground", MaterialTheme.colorScheme.surface to fg),
            Triple("primary", "primary-foreground", Color(0xffffd600) to Color.Black),
            Triple("primary-hover", "primary-foreground", Color(0xffffe34d) to Color.Black),
            Triple("secondary", "secondary-foreground", muted to fg),
            Triple("muted", "muted-foreground", muted to mutedFg),
            Triple(
                "accent",
                "accent-foreground",
                (if (dark) Color(0xff2e2e2e) else Color(0xfffffbe8)) to
                    (if (dark) Color.White else Color(0xff424242)),
            ),
            Triple(
                "brand",
                "brand-foreground",
                (if (dark) Color(0xffd9d9d9) else Color(0xff626262)) to
                    (if (dark) Color.Black else Color.White),
            ),
            Triple(
                "brand-muted",
                "brand-muted-foreground",
                (if (dark) Color(0xff303030) else Color(0xfffff6cc)) to
                    (if (dark) Color(0xffe5e5e5) else Color.Black),
            ),
        )
    Column(
        Modifier.fillMaxWidth().padding(24.dp),
        verticalArrangement = Arrangement.spacedBy(24.dp),
    ) {
        Text("Design tokens", weight = FontWeight.SemiBold, size = 24.sp, lineHeight = 32.sp)
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text(
                "Zoo · Zli Yellow",
                Modifier.onGloballyPositioned {
                    brandTop = with(density) { it.positionInRoot().y.toDp().value }
                },
                weight = FontWeight.SemiBold,
                size = 18.sp,
                lineHeight = 28.sp,
            )
            Text("#FFD600 / #000000 / #FFFFFF", muted = true)
            CssFlowRow(
                horizontalGap = 16.dp,
                verticalGap = 16.dp,
                verticalAlignment = Alignment.Bottom,
            ) {
                listOf(16, 32, 64, 192).forEach { size ->
                    var captionAdvance by remember(size) { mutableStateOf(Float.NaN) }
                    val imageWidth = with(density) { size.dp.toPx() }
                    Column(
                        Modifier.cssPreferredWidth(maxOf(imageWidth, captionAdvance)),
                        verticalArrangement = Arrangement.spacedBy(4.dp),
                    ) {
                        ZooMark(size.dp)
                        Text(
                            "${size}px",
                            small = true,
                            muted = true,
                            onTextLayout = {
                                if (it.lineCount > 0)
                                    captionAdvance = it.getLineRight(0) - it.getLineLeft(0)
                            },
                        )
                    }
                }
            }
            CssFlowRow(
                horizontalGap = 12.dp,
                verticalGap = 12.dp,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Button("メッセージを送信", primary = true) {}
                Button("キャンセル", variant = "outline") {}
                CompositionLocalProvider(
                    LocalMfmStyle provides
                        LocalMfmStyle.current.copy(textDecoration = TextDecoration.Underline)
                ) {
                    Text(
                        "本文内のリンク",
                        Modifier.cssPreferredWidth(linkAdvance)
                            .then(webControlModifier("link"))
                            .clickable { app.platform.action("catalog-scroll:$brandTop") },
                        color = LocalLinkColor.current,
                        size = 16.sp,
                        lineHeight = 24.sp,
                        onTextLayout = {
                            if (it.lineCount > 0)
                                linkAdvance = it.getLineRight(0) - it.getLineLeft(0)
                        },
                    )
                }
                Text(
                    "選択中のチャンネル",
                    Modifier.cssPreferredWidth(badgeAdvance + with(density) { 24.dp.toPx() })
                        .background(pairs.last().third.first, RoundedCornerShape(10.dp))
                        .padding(horizontal = 12.dp, vertical = 8.dp),
                    color = pairs.last().third.second,
                    onTextLayout = {
                        if (it.lineCount > 0) badgeAdvance = it.getLineRight(0) - it.getLineLeft(0)
                    },
                )
            }
        }
        BoxWithConstraints(Modifier.fillMaxWidth()) {
            val columns = if (maxWidth >= 592.dp) 2 else 1
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                pairs.chunked(columns).forEach { row ->
                    Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        row.forEach { (background, foreground, colors) ->
                            Column(
                                Modifier.weight(1f)
                                    .background(colors.first, RoundedCornerShape(10.dp))
                                    .border(
                                        1.dp,
                                        MaterialTheme.colorScheme.outline,
                                        RoundedCornerShape(10.dp),
                                    )
                                    .padding(17.dp)
                            ) {
                                Text(
                                    background,
                                    weight = FontWeight.Medium,
                                    size = 16.sp,
                                    lineHeight = 24.sp,
                                    color = colors.second,
                                )
                                Text("$foreground · Aa あいう 中文 0123", color = colors.second)
                            }
                        }
                    }
                }
            }
        }
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("Typography", weight = FontWeight.SemiBold, size = 18.sp, lineHeight = 28.sp)
            listOf(
                    Triple("text-xs", 12, 16),
                    Triple("text-sm", 14, 20),
                    Triple("text-base", 16, 24),
                    Triple("text-lg", 18, 28),
                    Triple("text-2xl", 24, 32),
                )
                .forEach { (name, size, line) ->
                    Text("$name · Zooで会話を始める Aa 中文 0123", size = size.sp, lineHeight = line.sp)
                }
        }
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("Spacing", weight = FontWeight.SemiBold, size = 18.sp, lineHeight = 28.sp)
            listOf(1, 2, 3, 4, 6, 8).forEach { step ->
                Row(
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    val fonts = LocalFonts.current
                    CompositionLocalProvider(LocalFonts provides fonts.copy(latin = fonts.mono)) {
                        Text("$step × --spacing", size = 16.sp, lineHeight = 24.sp)
                    }
                    Box(Modifier.size((step * 4).dp, 16.dp).background(Color(0xffffd600)))
                }
            }
        }
    }
}
