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
import android.icu.text.BreakIterator
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
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.core.content.IntentCompat
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import zoo.LocalTouchUi
import zoo.LocalApp
import zoo.Avatar
import zoo.Text
import works.zli.zoo.R
import zoo.ZooTheme

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

internal fun incomingCallerInitials(name: String): String {
    val firstWord = name.trim().takeWhile { !it.isWhitespace() }
    val characters = BreakIterator.getCharacterInstance().apply { setText(firstWord) }
    repeat(2) {
        if (characters.next() == BreakIterator.DONE) return firstWord.uppercase()
    }
    return firstWord.substring(0, characters.current()).uppercase()
}

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
            CompositionLocalProvider(LocalTouchUi provides true) {
                ZooTheme(dark = true) {
                    val colors = MaterialTheme.colorScheme
                    Surface(color = colors.background) {
                        BoxWithConstraints(
                            Modifier.fillMaxSize()
                                .background(Brush.verticalGradient(listOf(colors.primary.copy(alpha = 0.13f), colors.background, colors.background)))
                                .safeDrawingPadding(),
                            contentAlignment = Alignment.TopCenter,
                        ) {
                            val height = maxHeight
                            Column(
                                Modifier.widthIn(max = 440.dp).fillMaxWidth()
                                    .verticalScroll(rememberScrollState())
                                    .heightIn(min = height)
                                    .padding(horizontal = 24.dp, vertical = 24.dp),
                                horizontalAlignment = Alignment.CenterHorizontally,
                                verticalArrangement = Arrangement.spacedBy(32.dp),
                            ) {
                                Column(
                                    Modifier.weight(1f, fill = true).fillMaxWidth(),
                                    horizontalAlignment = Alignment.CenterHorizontally,
                                    verticalArrangement = Arrangement.Center,
                                ) {
                                    Box(
                                        Modifier.size(128.dp)
                                            .border(1.dp, colors.primary.copy(alpha = 0.24f), CircleShape)
                                            .background(colors.primary.copy(alpha = 0.08f), CircleShape)
                                            .clearAndSetSemantics {},
                                        contentAlignment = Alignment.Center,
                                    ) {
                                        when {
                                            caller != null && host != null -> CompositionLocalProvider(LocalApp provides host.app) {
                                                Avatar(caller, size = 88.dp)
                                            }
                                            callerName != null -> Box(
                                                Modifier.size(88.dp).background(colors.primary, CircleShape),
                                                contentAlignment = Alignment.Center,
                                            ) {
                                                Text(incomingCallerInitials(callerName),
                                                    color = colors.onPrimary, size = MaterialTheme.typography.headlineMedium.fontSize,
                                                    weight = FontWeight.SemiBold)
                                            }
                                            else -> Image(painterResource(R.drawable.zoo_launcher), contentDescription = null,
                                                modifier = Modifier.size(88.dp).clip(CircleShape))
                                        }
                                    }
                                    Spacer(Modifier.height(24.dp))
                                    Column(
                                        Modifier.semantics(mergeDescendants = true) { heading() },
                                        horizontalAlignment = Alignment.CenterHorizontally,
                                    ) {
                                        Text(callerName ?: "Zoo", color = colors.primary, weight = FontWeight.SemiBold,
                                            textAlign = TextAlign.Center, maxLines = 2)
                                        Spacer(Modifier.height(8.dp))
                                        Text(title, size = MaterialTheme.typography.headlineMedium.fontSize,
                                            lineHeight = MaterialTheme.typography.headlineMedium.lineHeight,
                                            weight = FontWeight.SemiBold, textAlign = TextAlign.Center)
                                    }
                                }
                                Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                                    Button(onClick = { answerWhenUnlocked(answer) },
                                        modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp)) {
                                        Text(answerLabel, color = colors.onPrimary, weight = FontWeight.SemiBold)
                                    }
                                    OutlinedButton(onClick = { dispatch(dismiss) },
                                        modifier = Modifier.fillMaxWidth().heightIn(min = 56.dp)) {
                                        Text(dismissLabel, color = colors.onSurface)
                                    }
                                }
                            }
                        }
                    }
                }
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
