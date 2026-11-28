package zoo.android.ui

import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.material3.BottomSheetDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Dp

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun ActionSheet(colors: AndroidColors, dismiss: () -> Unit, modifier: Modifier = Modifier,
    skipPartial: Boolean = false, maxWidth: Dp = BottomSheetDefaults.SheetMaxWidth,
    handleModifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    ModalBottomSheet(onDismissRequest = dismiss, modifier = modifier,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = skipPartial), sheetMaxWidth = maxWidth,
        containerColor = colors.background, contentColor = colors.foreground,
        dragHandle = { BottomSheetDefaults.DragHandle(handleModifier, color = colors.secondary) }, content = content)
}
