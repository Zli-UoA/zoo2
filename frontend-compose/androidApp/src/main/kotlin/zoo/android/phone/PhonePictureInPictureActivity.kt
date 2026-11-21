package zoo.android.phone

import android.app.PictureInPictureParams
import android.app.RemoteAction
import android.content.Context
import android.content.Intent
import android.content.res.Configuration
import android.graphics.drawable.Icon
import android.os.Bundle
import android.util.Rational
import android.widget.Toast
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
import works.zli.zoo.R
import zoo.phone.PhonePeerAvatar
import androidx.compose.ui.unit.dp
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.launch
import zoo.Button
import zoo.LocalTouchUi
import zoo.Text
import zoo.ZooTheme

/** A separate activity keeps the caller display and controls independent of the chat UI. */
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
        val call = checkNotNull(host.controller.call)
        inPictureInPicture = isInPictureInPictureMode
        enterOnResume = savedInstanceState == null && !inPictureInPicture
        val hangup = host.app.t("sip.hangup")
        val open = callActivity(this, host, id, "OPEN")
        val end = endCall(this, host, id)
        parameters = PictureInPictureParams.Builder()
            .setAspectRatio(Rational(1, 1))
            .setActions(listOf(RemoteAction(Icon.createWithResource(this, R.drawable.ic_phone_hangup), hangup, hangup, end)))
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
                        if (inPictureInPicture) {
                            BoxWithConstraints(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                                PhonePeerAvatar(host.app, call, minOf(maxWidth, maxHeight) * 0.64f)
                            }
                        } else Column(
                            Modifier.fillMaxSize().padding(12.dp),
                            horizontalAlignment = Alignment.CenterHorizontally,
                            verticalArrangement = Arrangement.Center,
                        ) {
                            Text(host.app.t("sip.status.active"), small = true)

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

    override fun onPostResume() {
        super.onPostResume()
        val params = parameters ?: return
        if (!enterOnResume || isFinishing) return
        enterOnResume = false
        if (!runCatching { enterPictureInPictureMode(params) }.getOrDefault(false)) {
            Toast.makeText(this, phoneLabel(this, "androidPhone.pipUnavailable",
                "Check Picture-in-picture in Android settings."), Toast.LENGTH_LONG).show()
            finish()
        }
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
