package zoo

import androidx.compose.animation.core.*
import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.unit.*
import kotlin.math.ceil
import kotlin.math.floor
import kotlinx.serialization.json.*

@Composable
fun NativeStatus(platform: Platform, state: Row) {
    val phase = state.text("status")
    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        Column(
            Modifier.widthIn(max = 448.dp).padding(24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            if (phase == "checking" || phase == "loading")
                Text(platform.translate("common.loading"))
            else {
                Row {
                    listOf(
                            "ja" to "日本語",
                            "en" to "English",
                            "zh-Hant" to "繁體中文",
                            "zh-Hans" to "简体中文",
                            "yue" to "廣東話",
                        )
                        .forEach { (key, label) ->
                            Button(label) { platform.action("locale:$key") }
                        }
                }
                Text(
                    platform.translate(
                        if (phase == "loggedOut") "auth.loggedOut" else "auth.failed"
                    ),
                    weight = FontWeight.SemiBold,
                    size = 18.sp,
                )
                Text(
                    if (phase == "loggedOut") platform.translate("auth.loggedOutHelp")
                    else state.text("statusMessage"),
                    muted = true,
                )
                Button(
                    platform.translate(if (phase == "loggedOut") "auth.login" else "auth.retry"),
                    primary = true,
                ) {
                    platform.action("retry")
                }
            }
        }
    }
}

@Composable
fun SyncStatus(app: Session, config: Row) {
    val defaults = app.config
    val mode = config.text("shapeMode").ifEmpty { "proxy" }
    val txid = config.text("txidMode").ifEmpty { "skip" }
    val user =
        if (config["oidc"] is JsonObject) app.t("sync.oidc")
        else if ("devUserId" in config) config.text("devUserId").ifEmpty { app.t("sync.unset") }
        else defaults.userId
    val values = listOf("Shape: $mode", "txid: $txid", "${app.t("sync.user")}: $user")
    val measurer = rememberTextMeasurer()
    val fonts = LocalFonts.current
    val style = MaterialTheme.typography.bodySmall
    val advances =
        values.map {
            measurer.measure(fontText(it, fonts), style, softWrap = false).getLineRight(0)
        }
    val background = MaterialTheme.colorScheme.surfaceVariant
    CssFlowRow(
        horizontalGap = 8.dp,
        verticalGap = 8.dp,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val density = androidx.compose.ui.platform.LocalDensity.current
        values.forEachIndexed { index, value ->
            Box(
                Modifier.cssPreferredWidth(advances[index] + with(density) { 16.dp.toPx() })
                    .drawBehind {
                        drawRoundRect(
                            background,
                            cornerRadius = CornerRadius(4.dp.toPx()),
                            size =
                                Size(minOf(size.width, advances[index] + 16.dp.toPx()), size.height),
                        )
                    }
                    .padding(horizontal = 8.dp, vertical = 4.dp)
            ) {
                Text(value, muted = true, small = true)
            }
        }
    }
}

@Composable
fun OfflineBanner(app: Session, state: Row, onRetry: (() -> Unit)? = null) {
    val offline = !state.flag("isOnline")
    if (!offline && state.text("persistenceStatus") != "error") return
    val colors = androidx.compose.material3.MaterialTheme.colorScheme
    val title = app.t(if (offline) "offline.bannerTitle" else "offline.persistenceErrorTitle")
    val description =
        if (offline) app.t("offline.bannerDescription")
        else state.text("persistenceError").ifEmpty { app.t("offline.persistenceErrorDescription") }
    val textMeasurer = rememberTextMeasurer()
    val fonts = LocalFonts.current
    val titleAdvance =
        textMeasurer
            .measure(
                fontText(title, fonts, FontWeight.Medium),
                MaterialTheme.typography.bodyMedium,
                softWrap = false,
            )
            .getLineRight(0)
    val descriptionAdvance =
        textMeasurer
            .measure(
                fontText(description, fonts),
                MaterialTheme.typography.bodySmall,
                softWrap = false,
            )
            .getLineRight(0)
    val columnAdvance = maxOf(titleAdvance, descriptionAdvance)
    Box(
        Modifier.fillMaxWidth()
            .padding(top = 8.dp, start = 12.dp, end = 12.dp)
            .then(webControlModifier("status")),
        contentAlignment = Alignment.TopCenter,
    ) {
        androidx.compose.material3.Surface(
            modifier =
                Modifier.cssShadowMd(androidx.compose.foundation.shape.RoundedCornerShape(10.dp)),
            shape = androidx.compose.foundation.shape.RoundedCornerShape(10.dp),
            color = colors.surface,
            border = androidx.compose.foundation.BorderStroke(1.dp, colors.outline),
            shadowElevation = 0.dp,
        ) {
            Layout(
                modifier =
                    Modifier.widthIn(max = 576.dp)
                        .padding(1.dp)
                        .padding(horizontal = 12.dp, vertical = 8.dp),
                content = {
                    Icon(
                        "cloud-off",
                        modifier = Modifier.padding(top = 2.dp).size(16.dp),
                        color = colors.onSurface,
                    )
                    Column {
                        Text(title, weight = FontWeight.Medium)
                        Text(description, small = true, muted = true)
                    }
                    Button(app.t("offline.retry"), variant = "outline", size = "sm") {
                        if (onRetry != null) onRetry() else app.platform.action("retryOffline")
                    }
                },
            ) { children, constraints ->
                val loose = constraints.copy(minWidth = 0, minHeight = 0)
                val icon = children[0].measure(loose)
                val button = children[2].measure(loose)
                val buttonWidth =
                    (children[2].parentData as? CssPreferredWidth)?.width ?: button.width.toFloat()
                val gap = 8.dp.toPx()
                val columnWidth =
                    minOf(
                        columnAdvance,
                        (constraints.maxWidth - icon.width - buttonWidth - gap * 2).coerceAtLeast(
                            0f
                        ),
                    )
                val column = children[1].measure(Constraints.fixedWidth(ceil(columnWidth).toInt()))
                val x = icon.width + gap + columnWidth + gap
                val buttonX = floor(x).toInt()
                val fraction = x - buttonX
                layout(
                    constraints.constrainWidth(ceil(x + buttonWidth).toInt()),
                    constraints.constrainHeight(maxOf(icon.height, column.height, button.height)),
                ) {
                    icon.placeRelative(0, 0)
                    column.placeRelative(icon.width + gap.toInt(), 0)
                    button.placeRelativeWithLayer(buttonX, 0) { translationX = fraction }
                }
            }
        }
    }
}

@Composable
fun ShellSkeleton() {
    val colors = androidx.compose.material3.MaterialTheme.colorScheme
    @Composable
    fun block(modifier: Modifier, radius: Dp = 8.dp) {
        SkeletonBlock(modifier, radius)
    }
    BoxWithConstraints(Modifier.fillMaxSize().background(colors.background)) {
        val wide = maxWidth >= 768.dp
        Row(Modifier.fillMaxSize()) {
            Box(
                Modifier.width(if (wide) 63.dp else 55.dp)
                    .fillMaxHeight()
                    .background(
                        if (colors.background == androidx.compose.ui.graphics.Color.Black)
                            androidx.compose.ui.graphics.Color(0xff141414)
                        else androidx.compose.ui.graphics.Color(0xfffafaf8)
                    )
            )
            androidx.compose.material3.VerticalDivider()
            if (wide) {
                Column(
                    Modifier.width(287.dp).fillMaxHeight().padding(12.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    block(Modifier.width(96.dp).height(24.dp))
                    repeat(6) { block(Modifier.fillMaxWidth().height(36.dp), 10.dp) }
                }
                androidx.compose.material3.VerticalDivider()
            }
            Column(Modifier.weight(1f)) {
                PaneHeader {
                    block(Modifier.size(32.dp), 10.dp)
                    Spacer(Modifier.width(12.dp))
                    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        block(Modifier.width(160.dp).height(16.dp))
                        block(Modifier.width(96.dp).height(12.dp))
                    }
                }
                androidx.compose.material3.HorizontalDivider()
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    block(Modifier.fillMaxWidth().height(64.dp), 10.dp)
                    block(Modifier.fillMaxWidth(.75f).height(64.dp), 10.dp)
                    block(Modifier.fillMaxWidth(.5f).height(64.dp), 10.dp)
                }
            }
        }
    }
}

@Composable
fun SkeletonBlock(modifier: Modifier, radius: Dp = 8.dp) {
    val app = LocalApp.current
    val opacity =
        if (app.env.reducedMotion) 1f
        else {
            val transition = rememberInfiniteTransition()
            val animated by
                transition.animateFloat(
                    1f,
                    .5f,
                    infiniteRepeatable(
                        tween(1000, easing = CubicBezierEasing(.4f, 0f, .6f, 1f)),
                        RepeatMode.Reverse,
                    ),
                )
            animated
        }
    Box(
        modifier
            .alpha(opacity)
            .background(
                androidx.compose.material3.MaterialTheme.colorScheme.surfaceVariant,
                androidx.compose.foundation.shape.RoundedCornerShape(radius),
            )
    )
}
