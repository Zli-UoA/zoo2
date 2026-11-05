package zoo.android

import android.Manifest
import android.app.*
import android.content.*
import android.content.pm.PackageManager
import android.os.Build
import android.net.Uri
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import zoo.*

/** Notifications use the live authenticated session; credentials are never put in intents. */
internal class AndroidNotifications(private val activity: Activity, private val app: Session) {
    private val manager = activity.getSystemService(NotificationManager::class.java)
    private val token = java.util.UUID.randomUUID().toString()
    private val inbox = NotificationInbox()
    private val shown = mutableMapOf<String, Row>()
    private val busy = mutableSetOf<String>()
    private val replied = mutableSetOf<String>()
    private val replyIds = mutableMapOf<String, String>()
    private val observer: Job

    init {
        active = this
        manager.activeNotifications.filter { it.tag?.startsWith("zoo:") == true }
            .forEach { manager.cancel(it.tag, it.id) }
        listOf(
            "mentions" to "androidNotifications.mentions",
            "messages" to "androidNotifications.messages",
            "reactions" to "androidNotifications.reactions",
        ).forEach { (id, label) ->
            manager.createNotificationChannel(NotificationChannel(
                "zoo.$id", app.t(label, "actor" to ""),
                if (id == "reactions") NotificationManager.IMPORTANCE_DEFAULT else NotificationManager.IMPORTANCE_HIGH,
            ).apply { enableVibration(true) })
        }
        observer = app.scope.launch {
            combine(app.repo.notifications.rows, app.repo.notifications.loading) { rows, loading -> rows to loading }
                .collectLatest { (rows, loading) ->
                if (loading) return@collectLatest
                val unread = rows.filterNot { it.flag("is_read") }.associateBy { it.text("id") }
                shown.keys.toList().filter { it !in unread }.forEach(::dismiss)
                unread.forEach { (id, row) -> if (id in shown) shown[id] = row }
                for (row in inbox.update(rows)) {
                    if (!app.env.visible && allowed() && row.text("channel_id").isNotEmpty()) {
                        shown[row.text("id")] = row
                        post(row)
                    }
                }
            }
        }
    }

    fun allowed(): Boolean = manager.areNotificationsEnabled() &&
        (Build.VERSION.SDK_INT < 33 || activity.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED)

    private fun intent(id: String, action: String): Intent =
        Intent(activity, NotificationActionReceiver::class.java)
            .setAction(action)
            .setData(Uri.parse("zoo-notification://$token/$id/$action"))
            .putExtra("session", token).putExtra("notification", id)

    private fun action(id: String, name: String, mutable: Boolean = false): PendingIntent =
        PendingIntent.getBroadcast(activity, 0, intent(id, name), PendingIntent.FLAG_UPDATE_CURRENT or
            if (mutable) PendingIntent.FLAG_MUTABLE else PendingIntent.FLAG_IMMUTABLE)

    private fun post(row: Row, error: String? = null) {
        if (!allowed()) return
        val id = row.text("id")
        val data = row.nested("display_data")
        val actor = author(data.nested("actor")).ifEmpty { "Zoo" }
        val channel = data.nested("channel").text("channel_name")
        val kind = row.text("notification_type")
        val channelId = when (kind) { "reaction" -> "reactions"; "dm" -> "messages"; else -> "mentions" }
        val body = data.nested("message").text("content").ifEmpty {
            app.t("notifications.$kind", "actor" to actor)
        }
        val destination = Intent(activity, MainActivity::class.java)
            .setAction("zoo.notification.OPEN")
            .setData(Uri.parse("zoo-notification://$token/$id/open"))
            .putExtra("session", token).putExtra("notification", id)
            .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        val builder = Notification.Builder(activity, "zoo.$channelId")
            .setSmallIcon(works.zli.zoo.R.drawable.ic_notification)
            .setContentTitle(if (channel.isEmpty()) actor else "$actor · $channel")
            .setContentText(error ?: body)
            .setStyle(Notification.BigTextStyle().bigText(error ?: body))
            .setContentIntent(PendingIntent.getActivity(activity, 0, destination,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
            .setAutoCancel(true).setOnlyAlertOnce(true)
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setCategory(Notification.CATEGORY_MESSAGE)
            .setDeleteIntent(action(id, "dismiss"))
            .addAction(Notification.Action.Builder(null, app.t("notifications.read"), action(id, "read")).build())
        if (kind != "reaction" && row.text("channel_id").isNotEmpty() && row.text("message_id").isNotEmpty()) {
            val input = RemoteInput.Builder("reply").setLabel(app.t("home.reply")).build()
            builder.addAction(Notification.Action.Builder(null, app.t("home.reply"), action(id, "reply", true))
                .addRemoteInput(input).setAllowGeneratedReplies(true).build())
        }
        try { manager.notify("zoo:$token:$id", 0, builder.build()) }
        catch (_: SecurityException) { /* Permission may be revoked while building the notification. */ }
    }

    fun open(intent: Intent) {
        if (intent.getStringExtra("session") != token) return
        val row = shown[intent.getStringExtra("notification")] ?: return
        app.navigate("/channels/${row.text("channel_id")}?message=${row.text("message_id")}")
        dismiss(row.text("id"))
    }

    private fun dismiss(id: String) {
        manager.cancel("zoo:$token:$id", 0)
        shown.remove(id)
    }

    suspend fun receive(intent: Intent) {
        if (intent.getStringExtra("session") != token) return
        val id = intent.getStringExtra("notification") ?: return
        val row = shown[id] ?: return
        if (!busy.add(id)) return
        try {
            when (intent.action) {
                "dismiss" -> { dismiss(id); return }
                "reply" -> {
                    val text = RemoteInput.getResultsFromIntent(intent)?.getCharSequence("reply")?.toString()?.trim()
                        ?.takeIf { it.isNotEmpty() } ?: return
                    if (id !in replied) {
                        try {
                            app.api.request("/api/messages", "POST", obj(
                                "id" to replyIds.getOrPut(id) { app.platform.uuid() },
                                "channel_id" to row.text("channel_id"),
                                "parent_id" to row.text("message_id"),
                                "content" to text,
                            ))
                            replied += id
                        } catch (e: CancellationException) { throw e }
                        catch (_: Exception) { post(row, app.t("common.writeError")); return }
                    }
                }
                "read" -> Unit
                else -> return
            }
            app.api.request("/api/notifications/$id/read", "PATCH", obj())
            dismiss(id)
        } catch (e: CancellationException) { throw e }
        catch (_: Exception) { post(row, app.t("notifications.readError")) }
        finally { busy.remove(id) }
    }

    fun close() {
        observer.cancel()
        shown.keys.toList().forEach(::dismiss)
        if (active === this) active = null
    }

    fun dispatch(intent: Intent, finished: () -> Unit) {
        app.scope.launch {
            withTimeout(9_000) { receive(intent) }
        }.invokeOnCompletion { finished() }
    }

    companion object {
        internal var active: AndroidNotifications? = null
            private set
    }
}

class NotificationActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val controller = AndroidNotifications.active ?: return
        val result = goAsync()
        controller.dispatch(intent) { result.finish() }
    }
}
