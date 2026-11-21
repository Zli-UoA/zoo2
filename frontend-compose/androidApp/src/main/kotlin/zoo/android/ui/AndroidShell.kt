package zoo.android.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

internal enum class TopPage(val title: String, val icon: UiIcon) {
    Home("ホーム", UiIcon.Home),
    Channels("チャンネル", UiIcon.Channels),
    Notifications("通知", UiIcon.Bell),
    Bubble("Bubble", UiIcon.Bubble),
    Settings("設定", UiIcon.Settings),
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
    val accent = Color(0xffffd400)
}

@Composable
internal fun AndroidShell(
    selected: TopPage?,
    dark: Boolean,
    onSelect: (TopPage) -> Unit,
    screen: @Composable ColumnScope.(AndroidColors) -> Unit,
) {
    val colors = AndroidColors(dark)
    MaterialTheme {
        Column(Modifier.fillMaxSize().background(colors.background).safeDrawingPadding()) {
            Column(Modifier.weight(1f).fillMaxWidth()) { screen(colors) }
            if (selected != null) {
                Row(Modifier.fillMaxWidth().background(colors.quietSurface).selectableGroup()) {
                    for (page in TopPage.entries) {
                        val active = page == selected
                        Column(
                            Modifier.weight(1f).heightIn(min = 60.dp)
                                .selectable(active, role = Role.Tab) { onSelect(page) },
                            horizontalAlignment = Alignment.CenterHorizontally,
                            verticalArrangement = Arrangement.Center,
                        ) {
                            Box(
                                Modifier.background(if (active) colors.headerLight else Color.Transparent, CircleShape)
                                    .padding(horizontal = 12.dp, vertical = 2.dp),
                            ) {
                                UiGlyph(page.icon, if (active) colors.foreground else colors.secondary)
                            }
                            Text(
                                page.title, color = if (active) colors.foreground else colors.secondary, fontSize = 11.sp,
                                fontWeight = if (active) FontWeight.SemiBold else FontWeight.Normal,
                            )
                        }
                    }
                }
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
) {
    Row(
        Modifier.fillMaxWidth().background(Brush.verticalGradient(listOf(colors.headerLight, colors.background)))
            .heightIn(min = 56.dp).padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        if (onBack != null) {
            UiIconButton(UiIcon.Back, "戻る", colors.foreground, onBack)
        } else ZooBadge(colors, Modifier.size(26.dp))
        Column(Modifier.weight(1f).then(
            if (onOpen == null) Modifier else Modifier.heightIn(min = 48.dp).clickable(onClick = onOpen),
        ), verticalArrangement = Arrangement.Center) {
            Text(title, color = colors.foreground, fontSize = if (description == null) 20.sp else 16.sp,
                fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
            if (!description.isNullOrEmpty()) {
                Text(description, color = colors.secondary, fontSize = 13.sp, maxLines = 1,
                    overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
            }
        }
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
