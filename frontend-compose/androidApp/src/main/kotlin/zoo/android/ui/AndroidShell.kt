package zoo.android.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.ui.platform.testTag
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ProvideTextStyle
import androidx.compose.material3.Text
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.intl.Locale
import androidx.compose.ui.text.intl.LocaleList
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

internal val LocalAndroidTranslation = staticCompositionLocalOf<(String, Array<out Pair<String, String>>) -> String> {
    { key, _ -> key }
}

@Composable
internal fun androidLabel(key: String, fallback: String, vararg args: Pair<String, String>): String =
    LocalAndroidTranslation.current(key, args).takeUnless { it == key } ?: fallback

internal enum class TopPage(val title: String, val icon: UiIcon, val translationKey: String) {
    Home("ホーム", UiIcon.Home, "home.title"),
    Channels("チャンネル", UiIcon.Channels, "channels.title"),
    Search("検索", UiIcon.TextSearch, "globalSearch.openNav"),
    Notifications("通知", UiIcon.Bell, "notifications.title"),
    Bubble("Bubble", UiIcon.Bubble, "bubbleChat.title"),
    Settings("設定", UiIcon.Settings, "settings.navTitle"),
}

internal class AndroidColors(dark: Boolean) {
    val background = if (dark) Color.Black else Color.White
    val foreground = if (dark) Color(0xfffaf9f7) else Color(0xff191816)
    val secondary = if (dark) Color(0xffaaa79b) else Color(0xff66635a)
    val quietSurface = if (dark) Color(0xff181712) else Color(0xfff7f5eb)
    val headerLight = if (dark) Color(0xff282103) else Color(0xfffff0b9)
    val inputSurface = if (dark) Color(0xff151515) else Color(0xfff7f7f7)
    val inputText = if (dark) Color(0xfff5f5f5) else Color(0xff202020)
    val inputHint = if (dark) Color(0xffa6a6a6) else Color(0xff616161)
    val inputBoundary = if (dark) Color(0xff363636) else Color(0xffdedede)
    val unread = if (dark) Color(0xffff9494) else Color(0xffb91c1c)
    val accent = Color(0xffffd400)
    val mentionSurface = if (dark) Color(0xff303030) else Color(0xfffff6cc)
    val mentionText = if (dark) Color(0xffe5e5e5) else Color.Black
    val suggestionSelected = if (dark) Color(0xff2e2e2e) else Color(0xfffffbe8)
    val messageMention = Color(0xfff0b232)
    val messageTarget = if (dark) Color(0xff6ea8ff) else Color(0xff3b82f6)
    /** Search terms in results (Web's `--highlight`): a paler yellow than [accent], readable on both themes. */
    val searchHighlight = if (dark) Color(0xff5c4a00) else Color(0xffffe680)
    val searchHighlightText = if (dark) Color.White else Color.Black
}

internal val entryTextSize = 16.sp

internal object AndroidWeights {
    val body = FontWeight.Normal
    val label = FontWeight.Medium
    val userName = FontWeight.Medium
    val entryAuthor = FontWeight.SemiBold
    val emphasis = FontWeight.SemiBold
    val unreadChannel = FontWeight.Bold
}

@Composable
internal fun AndroidShell(
    selected: TopPage?,
    dark: Boolean,
    onSelect: (TopPage) -> Unit,
    language: String? = null,
    /** 幅の広い画面のナビゲーションレールで選択中にする項目（子画面では親のトップ）。 */
    section: TopPage? = selected,
    /** 幅の広い画面でナビゲーションレールを出すか（全画面のプレビューなどでは出さない）。 */
    allowRail: Boolean = true,
    screen: @Composable ColumnScope.(AndroidColors) -> Unit,
) {
    val colors = AndroidColors(dark)
    val background = if (selected == TopPage.Notifications) {
        val statusBar = WindowInsets.statusBars.asPaddingValues().calculateTopPadding()
        val fadeHeight = with(LocalDensity.current) { (statusBar + 144.dp).toPx() }
        Brush.verticalGradient(listOf(colors.headerLight, colors.background), endY = fadeHeight)
    } else null
    val scheme = (if (dark) darkColorScheme() else lightColorScheme()).copy(
        primary = colors.accent,
        onPrimary = Color.Black,
        primaryContainer = colors.headerLight,
        onPrimaryContainer = colors.foreground,
        background = colors.background,
        surface = colors.background,
        onSurface = colors.foreground,
        surfaceVariant = colors.quietSurface,
        onSurfaceVariant = colors.secondary,
        outline = colors.inputHint,
        outlineVariant = colors.inputBoundary,
    )
    val locale = when (language) {
        null -> Locale.current
        "yue" -> Locale("yue-Hant-HK")
        else -> Locale(language)
    }
    MaterialTheme(colorScheme = scheme) {
        ProvideTextStyle(MaterialTheme.typography.bodyLarge.copy(
            fontFamily = FontFamily.SansSerif,
            localeList = LocaleList(locale),
        )) {
            BoxWithConstraints(Modifier.fillMaxSize()) {
            // 幅の広い画面（タブレット・開いた折りたたみ端末）では、下部のナビゲーションの代わりに左のレールを
            // 常に出す（iPad の左の列と同じ）。子画面でも親のトップを選択中にする。
            val rail = allowRail && maxWidth >= AndroidLayout.railMinWidth
            Row(Modifier.fillMaxSize()
                .then(if (background != null) Modifier.background(background) else Modifier.background(colors.background))
                .safeDrawingPadding()) {
              if (rail) NavigationRailColumn(section, colors, onSelect)
              Column(Modifier.weight(1f).fillMaxHeight()) {
                Column(Modifier.weight(1f).fillMaxWidth()) { screen(colors) }
                if (selected != null && !rail) {
                    Row(Modifier.fillMaxWidth().background(colors.quietSurface).selectableGroup()) {
                        for (page in TopPage.entries) {
                            val active = page == selected
                            Column(
                                Modifier.weight(1f).heightIn(min = AndroidControlSize.navigationHeight)
                                    .selectable(active, role = Role.Tab) { onSelect(page) },
                                horizontalAlignment = Alignment.CenterHorizontally,
                                verticalArrangement = Arrangement.Center,
                            ) {
                                Box(
                                    Modifier.background(if (active) colors.headerLight else Color.Transparent, CircleShape)
                                        .size(width = AndroidControlSize.target, height = AndroidControlSize.navigationIndicatorHeight),
                                    contentAlignment = Alignment.Center,
                                ) {
                                    UiGlyph(page.icon, if (active) colors.foreground else colors.secondary)
                                }
                                Text(
                                    androidLabel(page.translationKey, page.title), color = if (active) colors.foreground else colors.secondary, fontSize = 11.sp,
                                    fontWeight = if (active) AndroidWeights.label else AndroidWeights.body,
                                )
                            }
                        }
                    }
                }
              }
            }
            }
        }
    }
}

/** 幅の広い画面の基準（Material のウィンドウサイズクラスの medium と expanded）。 */
internal object AndroidLayout {
    /** これ以上の幅で、下部のナビゲーションを左のレールにする。 */
    val railMinWidth = 600.dp
    /** レールを除いた幅がこれ以上で、チャンネル一覧と会話を並べる（ウィンドウ幅 840dp − レール 80dp）。 */
    val listDetailMinWidth = 760.dp
    val railWidth = 80.dp
    val listPaneWidth = 320.dp
}

@Composable
private fun NavigationRailColumn(section: TopPage?, colors: AndroidColors, onSelect: (TopPage) -> Unit) {
    Column(
        // 横向きのスマートフォンなど高さが足りないときも、すべてのタブに届くよう縦にスクロールできるようにする。
        Modifier.width(AndroidLayout.railWidth).fillMaxHeight().background(colors.quietSurface)
            .verticalScroll(androidx.compose.foundation.rememberScrollState())
            .padding(vertical = 12.dp).selectableGroup().testTag("navigation-rail"),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        ZooBadge(colors, Modifier.padding(bottom = 12.dp).size(32.dp))
        for (page in TopPage.entries) {
            val active = page == section
            Column(
                Modifier.fillMaxWidth().heightIn(min = AndroidControlSize.navigationHeight)
                    .selectable(active, role = Role.Tab) { onSelect(page) },
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
            ) {
                Box(
                    Modifier.background(if (active) colors.headerLight else Color.Transparent, CircleShape)
                        .size(width = AndroidControlSize.target, height = AndroidControlSize.navigationIndicatorHeight),
                    contentAlignment = Alignment.Center,
                ) { UiGlyph(page.icon, if (active) colors.foreground else colors.secondary) }
                Text(androidLabel(page.translationKey, page.title), color = if (active) colors.foreground else colors.secondary,
                    fontSize = 11.sp, fontWeight = if (active) AndroidWeights.label else AndroidWeights.body, maxLines = 1)
            }
        }
    }
}

@Composable
internal fun PageHeader(
    title: String,
    colors: AndroidColors,
    description: String? = null,
    onBack: (() -> Unit)? = null,
    onOpen: (() -> Unit)? = null,
    titleWeight: FontWeight = AndroidWeights.emphasis,
    paintBackground: Boolean = true,
    actions: @Composable RowScope.() -> Unit = {},
) {
    Row(
        Modifier.fillMaxWidth()
            .then(if (paintBackground) Modifier.background(Brush.verticalGradient(listOf(colors.headerLight, colors.background))) else Modifier)
            .heightIn(min = 56.dp).padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        if (onBack != null) {
            UiIconButton(UiIcon.Back, androidLabel("home.back", "戻る"), colors.foreground, onBack)
        } else ZooBadge(colors, Modifier.size(26.dp))
        val titleAction = if (onOpen == null) Modifier else Modifier
            .heightIn(min = AndroidControlSize.target)
            .clickable(role = Role.Button, onClickLabel = "チャンネル設定を開く", onClick = onOpen)
        Row(
            Modifier.weight(1f).then(titleAction),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Column(Modifier.weight(1f)) {
                Text(title, color = colors.foreground, fontSize = if (onOpen == null) 20.sp else 16.sp,
                    fontWeight = titleWeight, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
                if (!description.isNullOrEmpty()) {
                    Text(description, color = colors.secondary, fontSize = 13.sp, maxLines = 1,
                        overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
                }
            }
            if (onOpen != null) UiGlyph(UiIcon.Next, colors.secondary, iconSize = AndroidControlSize.inlineIcon)
        }
        actions()
    }
}

@Composable
internal fun ZooBadge(colors: AndroidColors, modifier: Modifier = Modifier) {
    Canvas(modifier) {
        drawCircle(colors.accent)
        drawOval(Color.Black, Offset(size.width * .28f, size.height * .43f), Size(size.width * .46f, size.height * .40f))
        for ((x, y) in listOf(.29f to .37f, .50f to .27f, .71f to .37f)) {
            drawCircle(Color.Black, size.width * .105f, Offset(size.width * x, size.height * y))
        }
    }
}
