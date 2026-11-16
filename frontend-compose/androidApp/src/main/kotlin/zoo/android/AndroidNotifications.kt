package zoo.android

import android.Manifest
import android.app.*
import android.content.*
import android.content.pm.PackageManager
import android.os.Build
import android.net.Uri
import android.graphics.Bitmap
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.collectLatest
import zoo.*

/** Notifications use the live authenticated session; credentials are never put in intents. */
internal class AndroidNotifications(private val activity: Activity, private val app: Session) {
    private val manager = activity.getSystemService(NotificationManager::class.java)
    private val token = java.util.UUID.randomUUID().toString()
    private val inbox = NotificationInbox()
    private val shown = mutableMapOf<String, Row>()
    private val summaries = mutableSetOf<String>()
    private val errors = mutableMapOf<String, String>()
    private val avatars = linkedMapOf<String, Bitmap?>()
    private val avatarJobs = mutableMapOf<String, Job>()
    private val avatarSlots = Semaphore(2)
    private var closed = false
    private val actions = NotificationActions(
        shown = shown,
        request = { path, method, body -> app.api.request(path, method, body); Unit },
        uuid = app.platform::uuid,
        canReply = ::canReply,
        replyParent = ::replyParent,
        showError = { row, key ->
            errors[row.text("id")] = app.t(key)
            post(row)
        },
        dismiss = ::dismiss,
    )
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
            app.repo.notifications.synchronizedRows.collectLatest(::update)
        }
    }

    internal fun update(rows: List<Row>?) {
        if (closed || rows == null) return
        val unread = rows.filterNot { it.flag("is_read") }.associateBy { it.text("id") }
        shown.keys.toList().filter { it !in unread }.forEach(::dismiss)
        unread.forEach { (id, row) ->
            if (id in shown) {
                val previous = shown.put(id, row)
                if (previous != row) post(row)
            }
        }
        for (row in inbox.update(rows)) {
            if (!app.env.visible && allowed() && row.text("channel_id").isNotEmpty()) {
                shown[row.text("id")] = row
                post(row)
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

    private fun canReply(row: Row): Boolean {
        val channel = app.repo.channels.rows.value.find { it.text("id") == row.text("channel_id") }
        return channel != null && !channel.flag("encrypted") && !channel.present("archive_at") &&
            replyParent(row) != null
    }

    private fun replyParent(row: Row): String? =
        notificationReplyParent(row, app.repo.cachedMessages(row.text("channel_id")))

    private fun post(row: Row, error: String? = errors[row.text("id")]) {
        if (!allowed()) return
        val id = row.text("id")
        if (id !in shown) return
        val data = row.nested("display_data")
        val actor = notificationActor(row)
        val channel = data.nested("channel").text("channel_name")
        val kind = row.text("notification_type")
        val channelId = notificationChannel(row)
        val body = notificationBody(row) { key, name -> app.t(key, "actor" to name) }
        val avatar = avatars[notificationAvatar(row)]
        val self = app.repo.users.rows.value.find { it.text("id") == app.config.userId }
            ?.let(::author)?.ifEmpty { null } ?: "Zoo"
        val destination = Intent(activity, MainActivity::class.java)
            .setAction("zoo.notification.OPEN")
            .setData(Uri.parse("zoo-notification://$token/$id/open"))
            .putExtra("session", token).putExtra("notification", id)
            .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        val builder = Notification.Builder(activity, "zoo.$channelId")
            .setSmallIcon(works.zli.zoo.R.drawable.ic_notification)
            .setContentTitle(if (channel.isEmpty()) actor else "$actor · $channel")
            .setContentText(error ?: body)
            .setStyle(if (error != null) Notification.BigTextStyle().bigText(error)
                else notificationStyle(row, body, self, avatar))
            .setLargeIcon(avatar)
            .setWhen(timestampMillis(row.text("created_at")) ?: System.currentTimeMillis())
            .setGroup("zoo:$token:${notificationGroup(row)}")
            .setGroupAlertBehavior(Notification.GROUP_ALERT_CHILDREN)
            .setContentIntent(PendingIntent.getActivity(activity, 0, destination,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
            .setAutoCancel(true).setOnlyAlertOnce(true)
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setCategory(Notification.CATEGORY_MESSAGE)
            .setDeleteIntent(action(id, "dismiss"))
            .addAction(Notification.Action.Builder(null, app.t("notifications.read"), action(id, "read")).build())
        if (kind != "reaction" && !actions.hasReplied(id) && canReply(row)) {
            val input = RemoteInput.Builder("reply").setLabel(app.t("home.reply")).build()
            builder.addAction(Notification.Action.Builder(null, app.t("home.reply"), action(id, "reply", true))
                .addRemoteInput(input).setAllowGeneratedReplies(true).build())
        }
        try { manager.notify("zoo:$token:$id", 0, builder.build()) }
        catch (_: SecurityException) { /* Permission may be revoked while building the notification. */ }
        updateSummaries()
        loadAvatar(row)
    }

    private fun loadAvatar(row: Row) {
        val url = notificationAvatar(row)
        if (closed || url.isEmpty() || avatars.containsKey(url) || url in avatarJobs) return
        // ImageCache shares downloads with the UI. Limit decoding and keep thumbnails only.
        avatarJobs[url] = app.scope.launch {
            val bitmap = try {
                withTimeoutOrNull(8_000) {
                    avatarSlots.withPermit {
                        val bytes = app.image(url)
                        withContext(Dispatchers.Default) {
                            decodeOrientedBitmap(bytes, maxEdge = 256)
                        }
                    }
                }
            } catch (e: CancellationException) { throw e }
            catch (_: Exception) { null }
            avatarJobs.remove(url)
            if (closed) return@launch
            avatars[url] = bitmap
            while (avatars.size > 64) avatars.remove(avatars.keys.first())
            if (bitmap != null) shown.values.filter { notificationAvatar(it) == url }.toList().forEach { post(it) }
        }
    }

    private fun updateSummaries() {
        val groups = shown.values.groupBy(::notificationGroup).filterValues { it.size > 1 }
        (summaries - groups.keys).forEach { manager.cancel("zoo:$token:group:$it", 0) }
        summaries.retainAll(groups.keys)
        if (closed || !allowed()) return
        groups.forEach { (group, rows) ->
            val ordered = rows.sortedWith(compareBy<Row> { timestampMillis(it.text("created_at")) ?: 0L }
                .thenBy { it.text("id") })
            val latest = ordered.last()
            val title = latest.nested("display_data").nested("channel").text("channel_name")
                .ifEmpty { notificationActor(latest) }
            val style = Notification.InboxStyle().setBigContentTitle(title)
            ordered.takeLast(5).forEach { item ->
                val text = notificationBody(item) { key, actor -> app.t(key, "actor" to actor) }
                style.addLine("${notificationActor(item)}: $text")
            }
            val open = Intent(activity, MainActivity::class.java)
                .setAction("zoo.notification.OPEN")
                .setData(Uri.parse("zoo-notification://$token/${latest.text("id")}/open"))
                .putExtra("session", token).putExtra("notification", latest.text("id"))
                .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
            val summary = Notification.Builder(activity, "zoo.${notificationChannel(latest)}")
                .setSmallIcon(works.zli.zoo.R.drawable.ic_notification)
                .setContentTitle(title).setNumber(rows.size).setStyle(style)
                .setContentIntent(PendingIntent.getActivity(activity, 0, open,
                    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
                .setGroup("zoo:$token:$group").setGroupSummary(true)
                .setGroupAlertBehavior(Notification.GROUP_ALERT_CHILDREN)
                .setOnlyAlertOnce(true).setVisibility(Notification.VISIBILITY_PRIVATE)
                .setDeleteIntent(action(group, "dismissGroup"))
                .build()
            try {
                manager.notify("zoo:$token:group:$group", 0, summary)
                summaries += group
            } catch (_: SecurityException) { /* Permission may have changed. */ }
        }
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
        errors.remove(id)
        updateSummaries()
    }

    suspend fun receive(intent: Intent) {
        if (intent.getStringExtra("session") != token) return
        val id = intent.getStringExtra("notification") ?: return
        if (intent.action == "dismissGroup") {
            shown.values.filter { notificationGroup(it) == id }.map { it.text("id") }.forEach(::dismiss)
            return
        }
        val text = if (intent.action == "reply")
            RemoteInput.getResultsFromIntent(intent)?.getCharSequence("reply")?.toString() else null
        actions.receive(id, intent.action.orEmpty(), text)
    }

    fun close() {
        closed = true
        observer.cancel()
        avatarJobs.values.forEach { it.cancel() }
        avatarJobs.clear()
        avatars.clear()
        shown.keys.toList().forEach(::dismiss)
        if (active === this) active = null
    }

    fun dispatch(intent: Intent, finished: () -> Unit) {
        app.scope.launch {
            receive(intent)
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
