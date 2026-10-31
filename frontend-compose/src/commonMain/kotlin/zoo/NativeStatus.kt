package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*

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
