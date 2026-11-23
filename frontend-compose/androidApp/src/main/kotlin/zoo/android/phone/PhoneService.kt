package zoo.android.phone

import android.app.*
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.drawable.Icon
import android.net.Uri
import android.os.Build
import androidx.annotation.RequiresApi
import works.zli.zoo.R
import zoo.android.MainActivity
import zoo.android.PushBinding
import zoo.phone.PhoneCall

internal const val INCOMING_NOTIFICATION = 604
private const val CALL_NOTIFICATION = 600
internal const val INCOMING_CHANNEL = "zoo.phone.incoming"
// Reuse the existing channel to preserve notification settings across upgrades.
private const val CALL_CHANNEL = "zoo.phone.waiting"

/** The foreground service exists only while a call needs access to the microphone. */
class PhoneService : Service() {
    private var host: AndroidPhoneHost? = null
    private var callId: String? = null

    override fun onBind(intent: Intent?) = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val current = AndroidPhoneHost.current
        val id = intent?.getStringExtra(PHONE_CALL)
        if (current == null || id == null ||
            !current.matches(intent.getStringExtra(PHONE_SESSION), id) || current.audioCall(id) == null
        ) {
            if (host !== current || current?.audioCall(callId) == null) {
                runCatching { acknowledgeStart() }
                stopSelf(startId)
            }
            return START_NOT_STICKY
        }

        host = current
        callId = id
        if (refresh()) current.audioStarted(this, id)
        return START_NOT_STICKY
    }

    private fun acknowledgeStart() {
        val manager = getSystemService(NotificationManager::class.java)
        if (manager.getNotificationChannel(CALL_CHANNEL) == null) {
            manager.createNotificationChannel(NotificationChannel(
                CALL_CHANNEL, "Zoo", NotificationManager.IMPORTANCE_LOW,
            ))
        }
        val builder = Notification.Builder(this, CALL_CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Zoo")
            .setCategory(Notification.CATEGORY_CALL)
            .setGroup("zoo.phone.starting")
            .setGroupSummary(true)
            .setGroupAlertBehavior(Notification.GROUP_ALERT_CHILDREN)
        if (Build.VERSION.SDK_INT >= 31) {
            builder.setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_DEFERRED)
        }
        val notification = builder.build()
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(CALL_NOTIFICATION, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_PHONE_CALL)
        } else startForeground(CALL_NOTIFICATION, notification)
    }

    internal fun refresh(): Boolean {
        val current = host?.takeIf { it === AndroidPhoneHost.current } ?: return false
        val call = current.audioCall(callId) ?: return false
        val manager = getSystemService(NotificationManager::class.java)
        return try {
            manager.createNotificationChannel(NotificationChannel(
                CALL_CHANNEL, current.app.t("androidPhone.nativeCalls"), NotificationManager.IMPORTANCE_LOW,
            ))
            val end = endCall(this, current, call.id)
            val builder = callNotification(this, current, call, CALL_CHANNEL)
            if (Build.VERSION.SDK_INT >= 31) builder.setStyle(Notification.CallStyle.forOngoingCall(person(call), end))
            else builder.addAction(action(current.app.t("sip.hangup"), end))

            val notification = builder.build()
            if (Build.VERSION.SDK_INT >= 30) startForeground(
                CALL_NOTIFICATION, notification,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_PHONE_CALL or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE,
            )
            else if (Build.VERSION.SDK_INT >= 29) {
                startForeground(CALL_NOTIFICATION, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_PHONE_CALL)
            } else startForeground(CALL_NOTIFICATION, notification)
            true
        } catch (_: RuntimeException) {
            // While-in-use microphone access may be denied for a background answer.
            runCatching { acknowledgeStart() }
            try {
                current.audioFailed(call.id)
            } finally {
                stopSelf()
            }
            false
        }
    }

    override fun onDestroy() {
        stopForeground(STOP_FOREGROUND_REMOVE)
        host?.serviceDestroyed(this)
        super.onDestroy()
    }
}

internal fun showIncomingCall(context: Context, host: AndroidPhoneHost, call: PhoneCall, tag: String? = null) {
    val manager = context.getSystemService(NotificationManager::class.java)
    ensureIncomingChannel(context, host.app.t("sip.status.incoming"))
    val answer = callActivity(context, host, call.id, "ANSWER")
    val end = endCall(context, host, call.id)
    val answerAction = action(host.app.t("sip.answer"), answer)
    val endAction = action(host.app.t("sip.hangup"), end)
    val builder = callNotification(context, host, call, INCOMING_CHANNEL)
    incomingPresentation(context, builder, tag, call.id, answer, end, endAction.title.toString(),
        callerName = call.label.takeIf { call.kind == "dm" }, nativeSession = host.token)
    if (Build.VERSION.SDK_INT >= 31) {
        builder.setStyle(Notification.CallStyle.forIncomingCall(person(call), end, answer))
    } else builder.setActions(answerAction, endAction)

    synchronized(PushBinding.lock) {
        if (tag != null && PushBinding(context).dmInviteEnded(tag.substringAfterLast(':'))) return
        postIncomingNotification(builder, answerAction, endAction) { manager.notify(tag, INCOMING_NOTIFICATION, it) }
    }
}

internal fun dismissIncomingCall(context: Context, tag: String? = null) {
    context.getSystemService(NotificationManager::class.java).cancel(tag, INCOMING_NOTIFICATION)
    context.sendBroadcast(Intent(INCOMING_CLOSED_ACTION).setPackage(context.packageName).putExtra("tag", tag))
}

@RequiresApi(28)
private fun person(call: PhoneCall) = Person.Builder().setName(call.label.ifBlank { "Zoo" }).build()

private fun action(label: String, intent: PendingIntent) =
    Notification.Action.Builder(null as Icon?, label, intent).build()

private fun callNotification(context: Context, host: AndroidPhoneHost, call: PhoneCall, channel: String) =
    Notification.Builder(context, channel)
        .setSmallIcon(R.drawable.ic_notification)
        .setContentTitle(call.label.ifBlank { "Zoo" })
        .setContentText(host.app.t("sip.status.${call.status}"))
        .setContentIntent(callActivity(context, host, call.id, "OPEN"))
        .setCategory(Notification.CATEGORY_CALL)
        .setOngoing(true)
        .setOnlyAlertOnce(true)
        .setVisibility(Notification.VISIBILITY_PRIVATE)
        .setPublicVersion(Notification.Builder(context, channel).setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Zoo").setContentText(host.app.t("sip.title")).build())

private fun callIntent(context: Context, component: Class<*>, host: AndroidPhoneHost, id: String, action: String) =
    Intent(context, component).setAction("zoo.phone.$action")
        .setData(Uri.parse("zoo-phone://${host.token}/$id/$action"))
        .putExtra(PHONE_SESSION, host.token).putExtra(PHONE_CALL, id)

internal fun callActivity(context: Context, host: AndroidPhoneHost, id: String, action: String): PendingIntent =
    PendingIntent.getActivity(context, 0, callIntent(context, MainActivity::class.java, host, id, action)
        .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP), PendingIntent.FLAG_IMMUTABLE)

internal fun endCall(context: Context, host: AndroidPhoneHost, id: String): PendingIntent =
    PendingIntent.getBroadcast(context, 0, callIntent(context, PhoneActionReceiver::class.java, host, id, "END"),
        PendingIntent.FLAG_IMMUTABLE)

class PhoneActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action == DM_PUSH_DISMISS_ACTION) {
            dismissPushedDmCall(context, intent)
            return
        }
        if (intent.action != "zoo.phone.END") return
        val host = AndroidPhoneHost.current ?: return
        if (host.matches(intent.getStringExtra(PHONE_SESSION), intent.getStringExtra(PHONE_CALL))) {
            host.controller.hangup()
        }
    }
}

/** Some Android builds reject a separate CallStyle notification without a full-screen intent. */
internal fun postIncomingNotification(builder: Notification.Builder, answer: Notification.Action, end: Notification.Action, post: (Notification) -> Unit) {
    fun notification() = builder.build().apply { flags = flags or Notification.FLAG_INSISTENT }
    try { post(notification()) }
    catch (failure: IllegalArgumentException) {
        if (Build.VERSION.SDK_INT < 31) throw failure
        builder.setStyle(null).setActions(answer, end)
        post(notification())
    }
}
