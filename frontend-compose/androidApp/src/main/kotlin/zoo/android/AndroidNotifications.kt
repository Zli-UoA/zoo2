package zoo.android

import android.Manifest
import android.app.*
import android.content.*
import android.content.pm.PackageManager
import android.os.Build
import android.net.Uri
import android.graphics.Bitmap
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.snapshotFlow
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.collectLatest
import zoo.*

/** Notifications use the live authenticated session; credentials are never put in intents. */
internal class AndroidNotifications(
    private val activity: Activity,
    private val app: Session,
    private val avatarDispatcher: CoroutineDispatcher = Dispatchers.Default,
) {
    private val manager = activity.getSystemService(NotificationManager::class.java)
    private val token = java.util.UUID.randomUUID().toString()
    private val inbox = NotificationInbox()
    private val shown = mutableMapOf<String, Row>()
    private val summaries = mutableSetOf<String>()
    private val errors = mutableMapOf<String, String>()
    private val avatars = linkedMapOf<String, Bitmap>()
    private val avatarJobs = mutableMapOf<String, Job>()
    private val pictures = linkedMapOf<String, Bitmap?>()
    private val pendingPictures = linkedSetOf<String>()
    private val pictureJobs = mutableMapOf<String, Job>()
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
        dismiss = { dismiss(it) },
    )
    private val observer: Job
    private val pictureAccessObserver: Job
    private val pictureVersions = mutableMapOf<String, Long>()
    private data class PostedPicture(val attachmentId: String, val accessVersion: Long)
    private val postedPictures = mutableStateMapOf<String, PostedPicture>()

    init {
        active = this
        val prefix = "zoo:${app.config.userId}:"
        manager.activeNotifications.filter { notice ->
            notice.tag?.let { it.startsWith("zoo:") && (!it.startsWith(prefix) || it.startsWith("${prefix}group:")) } == true
        }.forEach { manager.cancel(it.tag, it.id) }
        DeliveryChannel.entries.forEach { it.register(activity, manager) }
        observer = app.scope.launch {
            app.repo.notifications.synchronizedRows.collectLatest(::update)
        }
        pictureAccessObserver = app.scope.launch {
            snapshotFlow {
                postedPictures.filterValues {
                    app.attachmentLinks.accessVersion(it.attachmentId) != it.accessVersion
                }
            }.collect { revoked ->
                for ((notificationId, picture) in revoked) {
                    if (postedPictures[notificationId] != picture) continue
                    val id = picture.attachmentId
                    if (pictureVersions[id] == picture.accessVersion) {
                        pictureVersions.remove(id)
                        pictures.remove(id)
                    }
                    shown[notificationId]?.let { post(it, refreshSummary = false, suppressPicture = true) }
                }
            }
        }
    }

    internal fun update(rows: List<Row>?) {
        if (closed || rows == null) return
        val unread = rows.filterNot { it.flag("is_read") }.associateBy { it.text("id") }
        val prefix = "zoo:${app.repo.config.userId}:"
        manager.activeNotifications.filter { it.tag?.startsWith(prefix) == true && it.tag?.startsWith("${prefix}group:") != true && it.tag?.removePrefix(prefix) !in unread }
            .forEach { manager.cancel(it.tag, it.id) }
        shown.keys.toList().filter { it !in unread }.forEach { dismiss(it, refreshSummary = false) }
        unread.forEach { (id, row) ->
            if (id in shown) {
                val previous = shown.put(id, row)
                if (previous != row) post(row, refreshSummary = false)
            }
        }
        for (row in inbox.update(rows)) {
            if (allowed() && row.text("channel_id").isNotEmpty() && row.text("notification_type") != "missed_call") {
                shown[row.text("id")] = row
                post(row, refreshSummary = false)
            }
        }
        updateSummaries()
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

    private fun previewImage(row: Row): Row? {
        if (row.text("notification_type") == "reaction") return null
        val id = row.text("channel_id")
        val channel = app.repo.channels.rows.value.find { it.text("id") == id } ?: return null
        return notificationPreviewImage(
            notificationCachedMessage(channel, row.text("message_id"), app.repo.cachedMessages(id)),
        )
    }

    private fun post(
        row: Row,
        error: String? = errors[row.text("id")],
        refreshSummary: Boolean = true,
        suppressPicture: Boolean = false,
    ) {
        if (!allowed()) return
        val id = row.text("id")
        if (id !in shown) return
        val data = row.nested("display_data")
        val actor = notificationActor(row)
        val channel = data.nested("channel").text("channel_name")
        val kind = row.text("notification_type")
        val channelId = notificationChannel(row)
        // Push で端末が復号した本文が同じ通知に出ていれば、それを残す（サーバーは暗号化チャンネルの本文を持たない。#510）。
        // 印は端末が復号したときだけ付けるので、チャンネルの行がまだ同期されていなくても使う。
        val tag = notificationTag(app.config.userId, id)
        val decrypted = preservedDecryptedBody(manager, tag)
        val body = decrypted ?: notificationBody(row) { key, name -> app.t(key, "actor" to name) }
        val avatar = avatars[notificationAvatar(row)]
        val image = if (suppressPicture) null else previewImage(row)
        val pictureId = image?.text("id")?.takeIf {
            !app.attachmentLinks.isDenied(it) && pictureVersions[it] == app.attachmentLinks.accessVersion(it)
        }
        val picture = pictureId?.let(pictures::get)
        val self = app.repo.users.rows.value.find { it.text("id") == app.config.userId }
            ?.let(::author)?.ifEmpty { null } ?: "Zoo"
        val destination = PushBinding(activity).openIntent(app.config.userId, id,
            "/channels/${row.text("channel_id")}?message=${row.text("message_id")}") ?: Intent(activity, MainActivity::class.java)
            .setAction("zoo.notification.OPEN")
            .setData(Uri.parse("zoo-notification://$token/$id/open"))
            .putExtra("session", token).putExtra("notification", id)
            .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        val builder = Notification.Builder(activity, "zoo.$channelId")
            .setSmallIcon(works.zli.zoo.R.drawable.ic_notification)
            .setContentTitle(if (channel.isEmpty()) actor else "$actor · $channel")
            .setContentText(error ?: body)
            .setStyle(when {
                error != null -> Notification.BigTextStyle().bigText(error)
                picture != null -> Notification.BigPictureStyle().bigPicture(picture).setSummaryText(body)
                else -> notificationStyle(row, body, self, avatar)
            })
            .setLargeIcon(avatar)
            .setWhen(timestampMillis(row.text("created_at")) ?: System.currentTimeMillis())
            .setGroup("zoo:${app.config.userId}:${notificationGroup(row)}")
            .setGroupAlertBehavior(Notification.GROUP_ALERT_CHILDREN)
            .setContentIntent(PendingIntent.getActivity(activity, 0, destination,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
            .setAutoCancel(true).setOnlyAlertOnce(true)
            .setVisibility(Notification.VISIBILITY_PRIVATE)
            .setCategory(if (kind == "reaction") Notification.CATEGORY_SOCIAL else Notification.CATEGORY_MESSAGE)
            .setExtras(android.os.Bundle().apply { decrypted?.let { putString(PUSH_DECRYPTED_BODY, it) } })
            .setDeleteIntent(action(id, "dismiss"))
            .addAction(Notification.Action.Builder(null, app.t("notifications.read"), action(id, "read")).build())
        if (kind != "reaction" && !actions.hasReplied(id) && canReply(row)) {
            val input = RemoteInput.Builder("reply").setLabel(app.t("home.reply")).build()
            builder.addAction(Notification.Action.Builder(null, app.t("home.reply"), action(id, "reply", true))
                .addRemoteInput(input).setAllowGeneratedReplies(true).build())
        }
        // Push と同じロックで、出す直前にもう一度確かめる（読んでから出すまでのあいだに Push が復号した本文を出したら、
        // それで上書きしない）。
        synchronized(PushBinding.lock) {
            val latest = preservedDecryptedBody(manager, tag)
            // エラーを出すときも、復号した本文の印は残す（次に出し直すときに本文へ戻すため）。
            (latest ?: decrypted)?.let { kept -> builder.setExtras(android.os.Bundle().apply { putString(PUSH_DECRYPTED_BODY, kept) }) }
            if (latest != null && latest != decrypted && error == null) {
                builder.setContentText(latest)
                    .setStyle(if (picture != null) Notification.BigPictureStyle().bigPicture(picture).setSummaryText(latest)
                        else notificationStyle(row, latest, self, avatar))
            }
            try {
                manager.notify(tag, 0, builder.build())
                if (error == null && picture != null && pictureId != null) {
                    postedPictures[id] = PostedPicture(pictureId, app.attachmentLinks.accessVersion(pictureId))
                } else {
                    postedPictures.remove(id)
                }
            } catch (_: SecurityException) { /* Permission may be revoked while building the notification. */ }
        }
        if (refreshSummary) updateSummaries()
        loadAvatar(row)
        if (error == null && image != null) loadPicture(image.text("id"))
    }

    private fun loadPicture(id: String) {
        if (closed || id.isBlank() || id in pictures || id in pictureJobs) return
        if (pictureJobs.size >= 2) {
            pendingPictures.add(id)
            return
        }
        pendingPictures.remove(id)
        val job = app.scope.launch(start = CoroutineStart.LAZY) {
            fun show(bitmap: Bitmap?) {
                if (closed) return
                pictures[id] = bitmap
                if (bitmap != null) pictureVersions[id] = app.attachmentLinks.accessVersion(id)
                while (pictures.size > 8) {
                    val oldest = pictures.keys.first()
                    pictures.remove(oldest)
                    pictureVersions.remove(oldest)
                }
                shown.values.filter { previewImage(it)?.text("id") == id }.toList()
                    .forEach { post(it, refreshSummary = false) }
            }

            suspend fun decode(bytes: ByteArray): Bitmap =
                withContext(avatarDispatcher) { decodeOrientedBitmap(bytes, maxEdge = 720) }

            val access = app.attachmentLinks
            val version = access.accessVersion(id)
            val key = "$id:original"
            val cached = if (access.isDenied(id)) null else try {
                app.platform.cachedAttachmentPreview(key)?.let { decode(it) }
            } catch (e: CancellationException) { throw e }
            catch (_: Exception) { null }
            if (cached != null && !access.isDenied(id) && access.accessVersion(id) == version) show(cached)
            val bitmap = try {
                withTimeoutOrNull(8_000) {
                    val url = access.get(id, force = true)
                    val authorizedVersion = access.accessVersion(id)
                    val image = if (cached != null && authorizedVersion == version) cached
                    else decode(app.image(url, signedAttachment = true, cacheKey = key))
                    image.takeIf { !access.isDenied(id) && access.accessVersion(id) == authorizedVersion }
                }
            } catch (e: CancellationException) { throw e }
            catch (_: Exception) { null }
            val visible = bitmap ?: cached?.takeIf { !access.isDenied(id) && access.accessVersion(id) == version }
            if (visible !== pictures[id] || id !in pictures) show(visible)
            pictureJobs.remove(id)
            pendingPictures.firstOrNull()?.let(::loadPicture)
        }
        pictureJobs[id] = job
        job.start()
    }

    private fun loadAvatar(row: Row) {
        val url = notificationAvatar(row)
        if (closed || url.isEmpty() || avatars.containsKey(url) || url in avatarJobs) return
        // ImageCache shares downloads with the UI. Limit decoding and keep thumbnails only.
        val job = app.scope.launch(start = CoroutineStart.LAZY) {
            val bitmap = try {
                withTimeoutOrNull(8_000) {
                    val bytes = app.image(url)
                    avatarSlots.withPermit {
                        withContext(avatarDispatcher) {
                            decodeOrientedBitmap(bytes, maxEdge = 256)
                        }
                    }
                }
            } catch (e: CancellationException) { throw e }
            catch (_: Exception) { null }
            avatarJobs.remove(url)
            if (closed || bitmap == null) return@launch
            avatars[url] = bitmap
            while (avatars.size > 64) avatars.remove(avatars.keys.first())
            // The summary has no avatar; only update the affected child notifications.
            shown.values.filter { notificationAvatar(it) == url }.toList()
                .forEach { post(it, refreshSummary = false) }
        }
        avatarJobs[url] = job
        job.start()
    }

    private fun updateSummaries() {
        val groups = shown.values.groupBy(::notificationGroup).filterValues { it.size > 1 }
        (summaries - groups.keys).forEach { manager.cancel("zoo:${app.config.userId}:group:$it", 0) }
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
                .setGroup("zoo:${app.config.userId}:$group").setGroupSummary(true)
                .setGroupAlertBehavior(Notification.GROUP_ALERT_CHILDREN)
                .setOnlyAlertOnce(true).setVisibility(Notification.VISIBILITY_PRIVATE)
                .setDeleteIntent(action(group, "dismissGroup"))
                .build()
            try {
                manager.notify("zoo:${app.config.userId}:group:$group", 0, summary)
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

    fun dismissTag(tag: String) {
        shown.keys.firstOrNull { notificationTag(app.config.userId, it) == tag }?.let { dismiss(it) }
    }

    private fun dismiss(id: String, refreshSummary: Boolean = true) {
        manager.cancel(notificationTag(app.repo.config.userId, id), 0)
        shown.remove(id)
        postedPictures.remove(id)
        errors.remove(id)
        if (refreshSummary) updateSummaries()
    }

    suspend fun receive(intent: Intent) {
        if (intent.getStringExtra("session") != token) return
        val id = intent.getStringExtra("notification") ?: return
        if (intent.action == "dismissGroup") {
            shown.values.filter { notificationGroup(it) == id }.map { it.text("id") }
                .forEach { dismiss(it, refreshSummary = false) }
            updateSummaries()
            return
        }
        val text = if (intent.action == "reply")
            RemoteInput.getResultsFromIntent(intent)?.getCharSequence("reply")?.toString() else null
        actions.receive(id, intent.action.orEmpty(), text)
    }

    fun close(preserveNotifications: Boolean = false) {
        closed = true
        observer.cancel()
        pictureAccessObserver.cancel()
        pictureVersions.clear()
        postedPictures.clear()
        avatarJobs.values.forEach { it.cancel() }
        avatarJobs.clear()
        pictureJobs.values.forEach { it.cancel() }
        pictureJobs.clear()
        pendingPictures.clear()
        pictures.clear()
        avatars.clear()
        // Push と同じロックの中で、今出ている通知を読んでから出し直す（あいだに Push が出した復号した本文を上書きしない）。
        if (preserveNotifications) synchronized(PushBinding.lock) {
            val posted = manager.activeNotifications.associateBy { it.tag }
            for (row in shown.values.toList()) {
                val id = row.text("id")
                val tag = notificationTag(app.config.userId, id)
                val destination = PushBinding(activity).openIntent(app.config.userId, id,
                    "/channels/${row.text("channel_id")}?message=${row.text("message_id")}")
                val notification = posted[tag]?.notification
                if (destination == null || notification == null) {
                    dismiss(id, refreshSummary = false)
                    continue
                }
                val detached = Notification.Builder.recoverBuilder(activity, notification)
                    .setActions().setDeleteIntent(null).setGroup(null)
                    .setContentIntent(PendingIntent.getActivity(activity, 0, destination,
                        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
                    .setOnlyAlertOnce(true).build().apply {
                        // recoverBuilder can retain the original array when its action list is empty.
                        actions = null
                    }
                try { manager.notify(tag, 0, detached) }
                catch (_: SecurityException) { manager.cancel(tag, 0) }
            }
            shown.clear()
        }
        else shown.keys.toList().forEach { dismiss(it, refreshSummary = false) }
        updateSummaries()
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
