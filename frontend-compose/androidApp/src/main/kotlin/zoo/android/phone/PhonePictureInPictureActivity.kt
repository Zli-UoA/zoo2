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
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.material3.Text
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import works.zli.zoo.R
import androidx.compose.ui.unit.dp
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.launch
import zoo.text
import zoo.android.ui.*

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
            val users by host.app.repo.users.replicatedRows.collectAsState()
            val members by host.app.repo.members.rows.collectAsState()
            val peerId = call.peerUserId ?: members.filter {
                it.text("channel_id") == call.target && it.text("user_id") != host.app.config.userId
            }.map { it.text("user_id") }.distinct().singleOrNull()
            val peer = users.firstOrNull { it.text("id") == peerId }
            val avatar = rememberCallImage(host.app, peer?.text("avatar_url"))
            val colors = AndroidColors(dark = true)
            BoxWithConstraints(Modifier.fillMaxSize().background(colors.background), contentAlignment = Alignment.Center) {
                if (inPictureInPicture) {
                    CallPortrait(AndroidScreen.Call(call.label, incoming = false, active = true, avatar = avatar,
                        hasUser = call.kind == "dm"), colors, minOf(maxWidth, maxHeight) * .64f)
                } else Column(
                    Modifier.safeDrawingPadding().padding(12.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(host.app.t("sip.status.active"), color = colors.foreground)
                    TextAction(host.app.t("androidPhone.expandCall"), colors, icon = UiIcon.Phone) {
                        runCatching { open.send() }
                        finish()
                    }
                    TextAction(hangup, colors, style = TextActionStyle.Neutral, icon = UiIcon.PhoneOff) {
                        runCatching { end.send() }
                    }
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        val session = intent.getStringExtra(PHONE_SESSION)
        val id = intent.getStringExtra(PHONE_CALL)
        val host = AndroidPhoneHost.current ?: return
        if (session != this.intent.getStringExtra(PHONE_SESSION) || id != this.intent.getStringExtra(PHONE_CALL) ||
            !host.matches(session, id) || host.controller.call?.status != "active"
        ) return

        setIntent(intent)
        enterOnResume = !isInPictureInPictureMode
    }

    override fun onPostResume() {
        super.onPostResume()
        val params = parameters ?: return
        if (!enterOnResume || isFinishing) return
        enterOnResume = false
        if (isInPictureInPictureMode) return
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
