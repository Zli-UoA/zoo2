package zoo.android.phone

import android.app.*
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.drawable.Icon
import android.media.AudioAttributes
import android.media.RingtoneManager
import android.os.*
import android.net.Uri
import zoo.android.MainActivity
import works.zli.zoo.R

/** User-enabled VoIP waiting, upgraded to a microphone/phone-call foreground service in a call. */
class PhoneService : Service() {
    private var host: AndroidPhoneHost? = null
    override fun onBind(intent: Intent?) = null
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val current = AndroidPhoneHost.current
        if (current == null) { stopSelf(); return START_NOT_STICKY }
        if (intent?.getStringExtra(PHONE_SESSION) != current.token) return START_NOT_STICKY
        host = current; current.service = this
        when (intent.action) {
            "zoo.phone.STOP" -> current.controller.stop()
            "zoo.phone.END" -> if (current.matches(current.token, intent.getStringExtra(PHONE_CALL))) current.controller.hangup()
        }
        if (current.desired) refresh() else stopSelf()
        return START_NOT_STICKY
    }
    fun refresh() {
        val current = host?.takeIf { it === AndroidPhoneHost.current } ?: return
        val app = current.app
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel("zoo.phone.waiting", app.t("androidPhone.nativeCalls"), NotificationManager.IMPORTANCE_LOW))
        manager.createNotificationChannel(NotificationChannel("zoo.phone.incoming", app.t("sip.status.incoming"), NotificationManager.IMPORTANCE_HIGH).apply {
            enableVibration(true)
            setSound(RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE), AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE).build())
        })
        val call = current.controller.call
        val open = Intent(this, MainActivity::class.java).setData(Uri.parse("zoo-phone://${current.token}/${call?.id ?: "waiting"}/open")).setAction("zoo.phone.OPEN").putExtra(PHONE_SESSION, current.token).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        call?.let { open.putExtra(PHONE_CALL, it.id) }
        val show = PendingIntent.getActivity(this, 601, open, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        fun end(): PendingIntent = PendingIntent.getService(this, 602,
            Intent(this, PhoneService::class.java).setData(Uri.parse("zoo-phone://${current.token}/${call?.id ?: "waiting"}/end")).setAction(if (call == null) "zoo.phone.STOP" else "zoo.phone.END").putExtra(PHONE_SESSION, current.token).putExtra(PHONE_CALL, call?.id),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val privateVersion = Notification.Builder(this, "zoo.phone.waiting").setSmallIcon(R.drawable.ic_notification).setContentTitle("Zoo").setContentText(app.t("sip.title")).build()
        val builder = Notification.Builder(this, "zoo.phone.waiting")
            .setSmallIcon(R.drawable.ic_notification).setContentTitle(call?.label?.ifBlank { "Zoo" } ?: "Zoo")
            .setContentText(if (call == null) app.t("androidPhone.waiting") else app.t("sip.status.${call.status}"))
            .setContentIntent(show).setOngoing(true).setOnlyAlertOnce(true)
            .setVisibility(Notification.VISIBILITY_PRIVATE).setPublicVersion(privateVersion)
        if (call != null) {
            builder.setCategory(Notification.CATEGORY_CALL)
            if (Build.VERSION.SDK_INT >= 31 && call.status != "incoming") builder.setStyle(Notification.CallStyle.forOngoingCall(Person.Builder().setName(call.label.ifBlank { "Zoo" }).build(), end()))
            else builder.addAction(Notification.Action.Builder(Icon.createWithResource(this, R.drawable.ic_notification), app.t("sip.hangup"), end()).build())
        } else builder.addAction(Notification.Action.Builder(Icon.createWithResource(this, R.drawable.ic_notification), app.t("sip.disconnect"), end()).build())
        try {
            if (Build.VERSION.SDK_INT >= 34) {
                val type = if (call != null && current.callAdmitted) ServiceInfo.FOREGROUND_SERVICE_TYPE_PHONE_CALL or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE else ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
                startForeground(600, builder.build(), type)
            } else if (Build.VERSION.SDK_INT >= 30 && call != null && current.callAdmitted) startForeground(600, builder.build(), ServiceInfo.FOREGROUND_SERVICE_TYPE_PHONE_CALL or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE)
            else if (Build.VERSION.SDK_INT >= 29 && call != null && current.callAdmitted) startForeground(600, builder.build(), ServiceInfo.FOREGROUND_SERVICE_TYPE_PHONE_CALL)
            else startForeground(600, builder.build())
        } catch (_: Exception) { current.controller.stop(); stopSelf(); return }
        if (call?.status == "incoming" && current.ringingId == call.id) {
            val answerIntent = Intent(open).putExtra("answer", true)
            val answer = PendingIntent.getActivity(this, 603, answerIntent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
            val ringing = Notification.Builder(this, "zoo.phone.incoming").setSmallIcon(R.drawable.ic_notification)
                .setContentTitle(call.label.ifBlank { "Zoo" }).setContentText(app.t("sip.status.incoming"))
                .setContentIntent(show).setCategory(Notification.CATEGORY_CALL).setOngoing(true).setOnlyAlertOnce(true)
                .setVisibility(Notification.VISIBILITY_PRIVATE).setPublicVersion(privateVersion)
            if (Build.VERSION.SDK_INT >= 31) ringing.setStyle(Notification.CallStyle.forIncomingCall(Person.Builder().setName(call.label.ifBlank { "Zoo" }).build(), end(), answer))
            else { ringing.addAction(Notification.Action.Builder(null as Icon?, app.t("sip.answer"), answer).build()); ringing.addAction(Notification.Action.Builder(null as Icon?, app.t("sip.hangup"), end()).build()) }
            manager.notify(604, ringing.build().apply { flags = flags or Notification.FLAG_INSISTENT })
        } else manager.cancel(604)
    }
    override fun onDestroy() {
        getSystemService(NotificationManager::class.java).cancel(604)
        stopForeground(STOP_FOREGROUND_REMOVE)
        host?.serviceDestroyed(this); host = null
        super.onDestroy()
    }
}
