package zoo.android.phone

import android.app.KeyguardManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioAttributes
import android.media.RingtoneManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.*
import androidx.compose.ui.Modifier
import androidx.core.content.ContextCompat
import androidx.core.content.IntentCompat
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import zoo.takeGraphemes
import zoo.text
import zoo.android.ui.AndroidColors
import zoo.android.ui.AndroidScreen
import zoo.android.ui.AndroidUiAction
import zoo.android.ui.CallPage

internal const val INCOMING_CLOSED_ACTION = "zoo.phone.INCOMING_CLOSED"

internal fun ensureIncomingChannel(context: Context, name: String) {
    val manager = context.getSystemService(NotificationManager::class.java)
    manager.createNotificationChannel(NotificationChannel(
        INCOMING_CHANNEL, name, NotificationManager.IMPORTANCE_HIGH,
    ).apply {
        enableVibration(true)
        vibrationPattern = longArrayOf(0, 400, 200, 400)
        setSound(RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE),
            AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE).build())
    })
}

internal fun incomingPresentation(
    context: Context,
    builder: Notification.Builder,
    tag: String?,
    callId: String,
    answer: PendingIntent,
    dismiss: PendingIntent,
    dismissLabel: String,
    timeoutMillis: Long = 0,
    callerName: String? = null,
    nativeSession: String? = null,
) {
    builder.setOngoing(true)
    val screen = Intent(context, IncomingCallActivity::class.java)
        .setData(Uri.parse("zoo-phone://incoming/$callId"))
        .putExtra("tag", tag).putExtra("answer", answer).putExtra("dismiss", dismiss)
        .putExtra("dismissLabel", dismissLabel)
        .putExtra("callerName", callerName)
        .putExtra(PHONE_CALL, callId).putExtra(PHONE_SESSION, nativeSession)
        .putExtra("deadline", if (timeoutMillis > 0) System.currentTimeMillis() + timeoutMillis else 0L)
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_NO_USER_ACTION)
    builder.setFullScreenIntent(PendingIntent.getActivity(
        context, 0, screen, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    ), true)
}

internal fun incomingCallerInitials(name: String): String =
    takeGraphemes(name.trim().takeWhile { !it.isWhitespace() }, 2).uppercase()

/** A lock-screen surface contains call controls only. The main app opens after unlocking. */
class IncomingCallActivity : ComponentActivity() {
    private val closed = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.getStringExtra("tag") == this@IncomingCallActivity.intent.getStringExtra("tag")) finish()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val answer = IntentCompat.getParcelableExtra(intent, "answer", PendingIntent::class.java)
        val dismiss = IntentCompat.getParcelableExtra(intent, "dismiss", PendingIntent::class.java)
        if (answer == null || dismiss == null) {
            finish()
            return
        }
        if (Build.VERSION.SDK_INT < 27) {
            @Suppress("DEPRECATION")
            window.addFlags(WindowManager.LayoutParams.FLAG_SHOW_WHEN_LOCKED or WindowManager.LayoutParams.FLAG_TURN_SCREEN_ON)
        }
        ContextCompat.registerReceiver(this, closed, IntentFilter(INCOMING_CLOSED_ACTION), ContextCompat.RECEIVER_NOT_EXPORTED)
        if (!hasIncomingNotification()) {
            finish()
            return
        }
        enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.dark(android.graphics.Color.TRANSPARENT),
            navigationBarStyle = SystemBarStyle.dark(android.graphics.Color.TRANSPARENT),
        )
        val deadline = intent.getLongExtra("deadline", 0)
        if (deadline > 0L) lifecycleScope.launch {
            delay((deadline - System.currentTimeMillis()).coerceAtLeast(0L))
            finish()
        }
        val title = phoneLabel(this, "sip.status.incoming", "Incoming call")
        val answerLabel = phoneLabel(this, "sip.answer", "Answer")
        val dismissLabel = intent.getStringExtra("dismissLabel") ?: phoneLabel(this, "common.close", "Close")
        val callerName = intent.getStringExtra("callerName")?.trim()?.takeIf { it.isNotEmpty() }
        val callId = intent.getStringExtra(PHONE_CALL)
        val host = AndroidPhoneHost.current?.takeIf { it.matches(intent.getStringExtra(PHONE_SESSION), callId) }
        val caller = callId?.let { host?.incomingCaller(it) }
        setContent {
            val avatar = rememberCallImage(host?.app, caller?.text("avatar_url"))
            val colors = AndroidColors(dark = true)
            val motion = android.provider.Settings.Global.getFloat(
                contentResolver, android.provider.Settings.Global.ANIMATOR_DURATION_SCALE, 1f,
            ) != 0f
            Box(Modifier.fillMaxSize().background(colors.background).safeDrawingPadding()) {
                CallPage(
                    AndroidScreen.Call(callerName ?: "Zoo", incoming = true, avatar = avatar,
                        hasUser = callerName != null, status = title),
                    colors, motion, { 0f }, dispatch = { action ->
                        when (action) {
                            AndroidUiAction.Answer -> answerWhenUnlocked(answer)
                            AndroidUiAction.HangUp -> dispatch(dismiss)
                            else -> Unit
                        }
                    }, answerLabel = answerLabel, dismissLabel = dismissLabel,
                )
            }
        }
    }

    private fun hasIncomingNotification(): Boolean {
        val tag = intent.getStringExtra("tag")
        return getSystemService(NotificationManager::class.java).activeNotifications.any {
            it.id == INCOMING_NOTIFICATION && it.tag == tag
        }
    }

    private fun answerWhenUnlocked(answer: PendingIntent) {
        val keyguard = getSystemService(KeyguardManager::class.java)
        if (!keyguard.isKeyguardLocked) dispatch(answer)
        else keyguard.requestDismissKeyguard(this, object : KeyguardManager.KeyguardDismissCallback() {
            override fun onDismissSucceeded() { if (!isFinishing) dispatch(answer) }
        })
    }

    private fun dispatch(action: PendingIntent) {
        if (hasIncomingNotification()) {
            try { action.send() } catch (_: PendingIntent.CanceledException) { }
        }
        finish()
    }

    override fun onDestroy() {
        runCatching { unregisterReceiver(closed) }
        super.onDestroy()
    }
}
