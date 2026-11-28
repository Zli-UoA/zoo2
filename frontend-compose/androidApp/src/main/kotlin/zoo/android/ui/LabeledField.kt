package zoo.android.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.LocalTextStyle
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

@Composable
internal fun RecoveryKeyField(label: String, value: String, enabled: Boolean, colors: AndroidColors, change: (String) -> Unit) {
    LabeledField(label, value, colors, enabled = enabled, transformation = PasswordVisualTransformation(),
        keyboard = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false), change = change)
}

@Composable
internal fun LabeledField(label: String, value: String, colors: AndroidColors, singleLine: Boolean = true,
    enabled: Boolean = true, transformation: VisualTransformation = VisualTransformation.None,
    keyboard: KeyboardOptions = KeyboardOptions.Default, change: (String) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(label, color = colors.secondary, fontSize = 13.sp)
        BasicTextField(value, change,
            Modifier.fillMaxWidth().background(colors.inputSurface, RoundedCornerShape(10.dp))
                .padding(12.dp).semantics { contentDescription = label },
            enabled = enabled, singleLine = singleLine, minLines = if (singleLine) 1 else 3,
            visualTransformation = transformation, keyboardOptions = keyboard,
            textStyle = LocalTextStyle.current.copy(color = colors.inputText, fontSize = 16.sp), cursorBrush = SolidColor(colors.inputText))
    }
}
