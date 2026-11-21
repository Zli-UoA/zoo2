package zoo.android.phone

import android.app.Notification
import android.app.NotificationManager
import android.app.Person
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.drawable.Icon
import android.net.Uri
import android.os.Build
import java.util.UUID
import works.zli.zoo.R
import zoo.android.MainActivity
import zoo.android.PushBinding
import zoo.android.notificationsAllowed

internal const val DM_PUSH_OPEN_ACTION = "zoo.phone.PUSH_OPEN"
internal const val DM_PUSH_ANSWER_ACTION = "zoo.phone.PUSH_ANSWER"
internal const val DM_PUSH_DISMISS_ACTION = "zoo.phone.PUSH_DISMISS"

internal fun dismissPushedDmCall(context: Context, intent: Intent) {
    val answer = DmCallPushAnswer.fromIntent(intent) ?: return
    val user = intent.getStringExtra("user") ?: return
    val store = PushBinding(context)
    synchronized(PushBinding.lock) {
        if (!store.matches(answer.binding, user)) return
        store.rememberEndedDmInvite(user, answer.id)
        dismissIncomingCall(context, dmNotificationTag(user, answer.id))
    }
}

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
    synchronized(PushBinding.lock) {
        if (store.matches(binding, user)) showBoundDmCallPush(context, store, user, binding, data)
    }
}

private fun showBoundDmCallPush(context: Context, store: PushBinding, user: String, binding: String, data: Map<String, String>) {
    val id = inviteUuid(data["inviteId"]) ?: return
    val manager = context.getSystemService(NotificationManager::class.java)
    val tag = dmNotificationTag(user, id)
    if (data["notificationType"] == "call_invite_end") {
        store.rememberEndedDmInvite(user, id)
        dismissIncomingCall(context, tag)
        return
    }
    if (!store.dmCallsEnabled(user) || store.dmInviteEnded(id) || !notificationsAllowed(context)) return
    val answer = DmCallPushAnswer(id, binding, data["expiresAt"]?.toLongOrNull() ?: return)
    val timeout = answer.remainingMillis()
    if (timeout == 0L || manager.activeNotifications.any { it.tag == tag && it.id == INCOMING_NOTIFICATION }) return

    ensureIncomingChannel(context, phoneLabel(context, "sip.status.incoming", "Incoming call"))
    val intent = Intent(context, MainActivity::class.java).setAction(DM_PUSH_ANSWER_ACTION)
        .setData(Uri.parse("zoo-phone://$binding/$id/push-answer"))
        .putExtra("invite", id).putExtra("binding", binding).putExtra("expiresAt", answer.expiresAt)
        .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
    val pending = PendingIntent.getActivity(context, 0, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    val openIntent = Intent(intent).setAction(DM_PUSH_OPEN_ACTION)
        .setData(Uri.parse("zoo-phone://$binding/$id/push-open"))
    val open = PendingIntent.getActivity(context, 0, openIntent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    val label = data["title"].orEmpty().take(200).ifBlank { "Zoo" }
    val dismissIntent = Intent(intent).setClass(context, PhoneActionReceiver::class.java)
        .setAction(DM_PUSH_DISMISS_ACTION).putExtra("user", user)
    val dismiss = PendingIntent.getBroadcast(context, 0, dismissIntent, PendingIntent.FLAG_IMMUTABLE)
    val answerAction = Notification.Action.Builder(null as Icon?,
        phoneLabel(context, "sip.answer", "Answer"), pending).build()
    val dismissAction = Notification.Action.Builder(null as Icon?,
        phoneLabel(context, "common.close", "Close"), dismiss).build()
    val builder = Notification.Builder(context, INCOMING_CHANNEL)
        .setSmallIcon(R.drawable.ic_notification)
        .setContentTitle(label)
        .setContentIntent(open)
        .setCategory(Notification.CATEGORY_CALL)
        .setTimeoutAfter(timeout).setOnlyAlertOnce(true)
        .setVisibility(Notification.VISIBILITY_PRIVATE)
        .setPublicVersion(Notification.Builder(context, INCOMING_CHANNEL)
            .setSmallIcon(R.drawable.ic_notification).setContentTitle("Zoo")
            .setContentText(phoneLabel(context, "sip.status.incoming", "Incoming call")).build())
    incomingPresentation(context, builder, tag, id, pending, dismiss, dismissAction.title.toString(), timeout,
        callerName = label)
    if (Build.VERSION.SDK_INT >= 31) {
        builder.setStyle(Notification.CallStyle.forIncomingCall(Person.Builder().setName(label).build(), dismiss, pending))
    } else builder.setActions(answerAction, dismissAction)
    try {
        postIncomingNotification(builder, answerAction, dismissAction) { manager.notify(tag, INCOMING_NOTIFICATION, it) }
    } catch (_: SecurityException) { /* Permission may change during delivery. */ }
}


internal fun phoneDictionaryLocale(context: Context): String {
    PushBinding(context).phoneLocale()?.let { return it }
    val locale = context.resources.configuration.locales[0] ?: return "en"
    return when (locale.language) {
        "ja", "yue" -> locale.language
        "ko" -> if (locale.country == "KP") "ko-KP" else "ko-KR"
        "zh" -> if (locale.script == "Hant" || locale.country in setOf("TW", "HK", "MO")) "zh-Hant" else "zh-Hans"
        else -> "en"
    }
}

internal fun phoneLabel(context: Context, key: String, fallback: String): String {
    val dictionaryLocale = phoneDictionaryLocale(context)
    return runCatching {
        val dictionary = phoneDictionaries.getOrPut(dictionaryLocale) {
            context.assets.open("locales/$dictionaryLocale.json")
                .bufferedReader().use { zoo.json.parseToJsonElement(it.readText()) as zoo.Row }
        }
        zoo.android.translateDictionary(dictionary, dictionaryLocale, key, emptyMap())
    }.getOrDefault(fallback)
}

private val phoneDictionaries = java.util.concurrent.ConcurrentHashMap<String, zoo.Row>()
