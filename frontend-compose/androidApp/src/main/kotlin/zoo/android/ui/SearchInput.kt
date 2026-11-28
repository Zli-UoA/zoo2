package zoo.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

@Composable
internal fun SearchInput(
    value: String, change: (String) -> Unit, hint: String, colors: AndroidColors,
    modifier: Modifier = Modifier, submit: (() -> Unit)? = null,
) {
    BasicTextField(value, change, modifier.fillMaxWidth().semantics { contentDescription = hint },
        singleLine = true, textStyle = LocalTextStyle.current.copy(color = colors.inputText, fontSize = 16.sp),
        cursorBrush = SolidColor(colors.inputText),
        keyboardOptions = if (submit == null) KeyboardOptions.Default else KeyboardOptions(imeAction = ImeAction.Search),
        keyboardActions = if (submit == null) KeyboardActions.Default else KeyboardActions(onSearch = { submit() }),
        decorationBox = { field ->
            Row(Modifier.fillMaxWidth().heightIn(min = AndroidControlSize.target)
                .background(colors.inputSurface, RoundedCornerShape(14.dp)).padding(horizontal = 12.dp),
                verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                UiGlyph(UiIcon.Search, colors.inputHint, AndroidControlSize.inlineIcon)
                Box(Modifier.weight(1f)) {
                    if (value.isEmpty()) Text(hint, color = colors.inputHint, fontSize = 16.sp, maxLines = 1)
                    field()
                }
                if (value.isNotEmpty()) UiIconButton(UiIcon.Close, "検索をクリア", colors.secondary, { change("") },
                    iconSize = AndroidControlSize.inlineIcon, modifier = Modifier.size(32.dp))
            }
        })
}
