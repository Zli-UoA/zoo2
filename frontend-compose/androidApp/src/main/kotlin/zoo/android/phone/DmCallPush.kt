package zoo.android.phone

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.drawable.Icon
import android.media.AudioAttributes
import android.media.RingtoneManager
import android.net.Uri
import java.util.UUID
import works.zli.zoo.R
import zoo.android.MainActivity
import zoo.android.PushBinding
import zoo.android.notificationsAllowed

internal const val DM_PUSH_ANSWER_ACTION = "zoo.phone.PUSH_ANSWER"

private fun inviteUuid(value: String?): String? =
    value?.takeIf { it.length == 36 }?.let { runCatching { UUID.fromString(it).toString() }.getOrNull() }

internal fun dmNotificationTag(user: String, inviteId: String) = "zoo:$user:call:$inviteId"

internal data class DmCallPushAnswer(val id: String, val binding: String, val expiresAt: Long) {
    fun remainingMillis() = (expiresAt - System.currentTimeMillis() / 1000L).coerceIn(0L, 30L) * 1000L

    companion object {
        fun fromIntent(intent: Intent): DmCallPushAnswer? {
            val id = inviteUuid(intent.getStringExtra("invite")) ?: return null
            val binding = intent.getStringExtra("binding") ?: return null
            return DmCallPushAnswer(id, binding, intent.getLongExtra("expiresAt", 0))
        }
    }
}

/** Handles the notification before an authenticated Activity exists. Only the live Shape can ring or answer. */
internal fun showDmCallPush(context: Context, store: PushBinding, user: String, binding: String, data: Map<String, String>) {
    val id = inviteUuid(data["inviteId"]) ?: return
    val manager = context.getSystemService(NotificationManager::class.java)
    val tag = dmNotificationTag(user, id)
    if (data["notificationType"] == "call_invite_end") {
        store.rememberEndedDmInvite(user, id)
        manager.cancel(tag, INCOMING_NOTIFICATION)
        return
    }
    if (!store.dmCallsEnabled(user) || store.dmInviteEnded(id) || !notificationsAllowed(context)) return
    val answer = DmCallPushAnswer(id, binding, data["expiresAt"]?.toLongOrNull() ?: return)
    val timeout = answer.remainingMillis()
    if (timeout == 0L || manager.activeNotifications.any { it.tag == tag && it.id == INCOMING_NOTIFICATION }) return

    if (manager.getNotificationChannel(INCOMING_CHANNEL) == null) {
        manager.createNotificationChannel(NotificationChannel(INCOMING_CHANNEL, "Zoo", NotificationManager.IMPORTANCE_HIGH).apply {
            enableVibration(true)
            setSound(RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE),
                AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE).build())
        })
    }
    val intent = Intent(context, MainActivity::class.java).setAction(DM_PUSH_ANSWER_ACTION)
        .setData(Uri.parse("zoo-phone://$binding/$id/push-answer"))
        .putExtra("invite", id).putExtra("binding", binding).putExtra("expiresAt", answer.expiresAt)
        .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
    val pending = PendingIntent.getActivity(context, 0, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    val notification = Notification.Builder(context, INCOMING_CHANNEL)
        .setSmallIcon(R.drawable.ic_notification)
        .setContentTitle(data["title"].orEmpty().take(200).ifBlank { "Zoo" })
        .addAction(Notification.Action.Builder(null as Icon?, answerLabel(context), pending).build())
        .setCategory(Notification.CATEGORY_CALL)
        .setTimeoutAfter(timeout).setOnlyAlertOnce(true)
        .setVisibility(Notification.VISIBILITY_PRIVATE)
        .build().apply { flags = flags or Notification.FLAG_INSISTENT }
    try { manager.notify(tag, INCOMING_NOTIFICATION, notification) }
    catch (_: SecurityException) { /* Permission may change during delivery. */ }
}

private fun answerLabel(context: Context): String {
    val locale = context.resources.configuration.locales[0] ?: return "Answer"
    val dictionaryLocale = when (locale.language) {
        "ja", "yue" -> locale.language
        "ko" -> if (locale.country == "KP") "ko-KP" else "ko-KR"
        "zh" -> if (locale.script == "Hant" || locale.country in setOf("TW", "HK", "MO")) "zh-Hant" else "zh-Hans"
        else -> "en"
    }
    return runCatching {
        val dictionary = context.assets.open("locales/$dictionaryLocale.json")
            .bufferedReader().use { zoo.json.parseToJsonElement(it.readText()) as zoo.Row }
        zoo.android.translateDictionary(dictionary, dictionaryLocale, "sip.answer", emptyMap())
    }.getOrDefault("Answer")
}
