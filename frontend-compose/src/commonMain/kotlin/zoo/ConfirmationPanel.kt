package zoo

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.*

class DialogAction(
    val label: String,
    val variant: String = "outline",
    val enabled: Boolean = true,
    val action: () -> Unit,
)

@Composable
fun DialogFooter(actions: List<DialogAction>, minimumActionHeight: Dp = 0.dp) {
    val width = LocalWindowInfo.current.containerSize.width
    val compact = with(LocalDensity.current) { width.toDp() < 640.dp }
    val border = MaterialTheme.colorScheme.outline
    val modifier =
        Modifier.fillMaxWidth()
            .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = .5f))
            .drawBehind { drawLine(border, Offset.Zero, Offset(size.width, 0f), 1.dp.toPx()) }
            .padding(start = 16.dp, end = 16.dp, bottom = 16.dp, top = 17.dp)
    if (compact)
        Column(modifier, verticalArrangement = Arrangement.spacedBy(8.dp)) {
            actions.reversed().forEach { item ->
                Button(
                    item.label,
                    item.enabled,
                    variant = item.variant,
                    modifier = Modifier.fillMaxWidth().heightIn(min = minimumActionHeight),
                    click = item.action,
                )
            }
        }
    else
        CssFlowRow(
            modifier,
            horizontalGap = 8.dp,
            alignEnd = true,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            actions.forEach { item ->
                Button(
                    item.label,
                    item.enabled,
                    variant = item.variant,
                    modifier = Modifier.heightIn(min = minimumActionHeight),
                    click = item.action,
                )
            }
        }
}

@Composable
fun ConfirmationPanel(
    title: String,
    description: String,
    action: String,
    busy: Boolean,
    error: String?,
    cancel: () -> Unit,
    confirm: () -> Unit,
) = LabeledConfirmationPanel(title, description, action, busy, error, null, "destructive", cancel, confirm)

/** 取り消しの文字と、確認ボタンの見た目を指定できる [ConfirmationPanel]（アプリのメッセージ部品の confirm）。 */
@Composable
fun LabeledConfirmationPanel(
    title: String,
    description: String,
    action: String,
    busy: Boolean,
    error: String?,
    cancelLabel: String?,
    confirmVariant: String,
    cancel: () -> Unit,
    confirm: () -> Unit,
) {
    val app = LocalApp.current
    Panel(
        title,
        { if (!busy) cancel() },
        alert = true,
        description = description,
        footer = {
            DialogFooter(
                listOf(
                    DialogAction(cancelLabel ?: app.t("common.cancel"), enabled = !busy, action = cancel),
                    DialogAction(
                        if (busy) app.t("moderation.executing") else action,
                        confirmVariant,
                        !busy,
                        confirm,
                    ),
                )
            )
        },
    ) {
        if (!error.isNullOrEmpty())
            Text(
                error,
                Modifier.then(webControlModifier("alert")).semantics {
                    liveRegion = LiveRegionMode.Polite
                },
                color = MaterialTheme.colorScheme.error,
            )
    }
}
