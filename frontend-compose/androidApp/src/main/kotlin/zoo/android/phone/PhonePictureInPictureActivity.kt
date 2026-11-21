package zoo.android.phone

import android.app.PictureInPictureParams
import android.app.RemoteAction
import android.content.Context
import android.content.Intent
import android.content.res.Configuration
import android.graphics.drawable.Icon
import android.os.Bundle
import android.util.Rational
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.launch
import zoo.Button
import zoo.LocalTouchUi
import zoo.Text
import zoo.ZooTheme

/** Only the call status is rendered here, including during the transition into PiP. */
class PhonePictureInPictureActivity : ComponentActivity() {
    private var inPictureInPicture by mutableStateOf(false)
    private var enterOnResume = true
    private var parameters: PictureInPictureParams? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val host = AndroidPhoneHost.current
        val id = intent.getStringExtra(PHONE_CALL)
        if (host == null || id == null || !host.matches(intent.getStringExtra(PHONE_SESSION), id) ||
            host.controller.call?.status != "active"
        ) {
            finish()
            return
        }
        inPictureInPicture = isInPictureInPictureMode
        enterOnResume = savedInstanceState == null && !inPictureInPicture
        val hangup = host.app.t("sip.hangup")
        val open = callActivity(this, host, id, "OPEN")
        val end = endCall(this, host, id)
        parameters = PictureInPictureParams.Builder()
            .setAspectRatio(Rational(2, 1))
            .setActions(listOf(RemoteAction(Icon.createWithResource(this, android.R.drawable.ic_menu_close_clear_cancel), hangup, hangup, end)))
            .build()
        lifecycleScope.launch {
            snapshotFlow { host.valid(id) && host.controller.call?.status == "active" }.collect { active ->
                if (!active) finish()
            }
        }
        setContent {
            CompositionLocalProvider(LocalTouchUi provides true) {
                ZooTheme(dark = true, locale = host.app.env.locale) {
                    Surface(color = MaterialTheme.colorScheme.surface) {
                        Column(
                            Modifier.fillMaxSize().padding(12.dp),
                            horizontalAlignment = Alignment.CenterHorizontally,
                            verticalArrangement = Arrangement.Center,
                        ) {
                            Text("Zoo", color = MaterialTheme.colorScheme.primary, weight = FontWeight.SemiBold)
                            Text(host.app.t("sip.status.active"), small = true)
                            if (!inPictureInPicture) {
                                Button(host.app.t("androidPhone.expandCall"), primary = true) {
                                    runCatching { open.send() }
                                    finish()
                                }
                                Button(hangup, destructive = true) { runCatching { end.send() } }
                            }
                        }
                    }
                }
            }
        }
    }

    override fun onPostResume() {
        super.onPostResume()
        val params = parameters ?: return
        if (!enterOnResume || isFinishing) return
        enterOnResume = false
        if (!runCatching { enterPictureInPictureMode(params) }.getOrDefault(false)) finish()
    }

    override fun onPictureInPictureModeChanged(isInPictureInPictureMode: Boolean, newConfig: Configuration) {
        super.onPictureInPictureModeChanged(isInPictureInPictureMode, newConfig)
        inPictureInPicture = isInPictureInPictureMode
    }

    companion object {
        internal fun open(context: Context, host: AndroidPhoneHost, id: String) {
            if (!host.valid(id) || host.controller.call?.status != "active") return
            context.startActivity(Intent(context, PhonePictureInPictureActivity::class.java)
                .putExtra(PHONE_SESSION, host.token).putExtra(PHONE_CALL, id)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
    }
}
