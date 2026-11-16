package zoo.android

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import com.google.android.gms.tasks.Task
import com.google.firebase.FirebaseApp
import com.google.firebase.messaging.FirebaseMessaging
import com.google.firebase.messaging.FirebaseMessagingService
import com.google.firebase.messaging.RemoteMessage
import java.util.UUID
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.collectLatest
import androidx.compose.runtime.snapshotFlow
import zoo.*

internal fun notificationTag(user: String, id: String) = "zoo:$user:$id"

/** No account credentials or message bodies are persisted. A new login gets a new delivery binding. */
internal class PushBinding(private val context: Context) {
    private val prefs = context.getSharedPreferences("push-binding", Context.MODE_PRIVATE)

    fun bind(base: String, user: String): String = synchronized(lock) {
        if (prefs.getString("base", null) == base && prefs.getString("user", null) == user) {
            prefs.getString("binding", null)?.let { return@synchronized it }
        }
        prefs.getString("user", null)?.let { clearNotifications(context, it) }
        UUID.randomUUID().toString().also {
            check(prefs.edit().clear().putString("base", base).putString("user", user)
                .putString("binding", it).commit())
        }
    }

    fun matches(binding: String?, user: String?): Boolean = synchronized(lock) {
        !binding.isNullOrEmpty() && !user.isNullOrEmpty() &&
            prefs.getString("binding", null) == binding && prefs.getString("user", null) == user
    }

    fun endpoint(binding: String): String? = if (prefs.getString("binding", null) == binding)
        prefs.getString("endpoint", null) else null

    fun registered(binding: String, endpoint: String) = synchronized(lock) {
        if (prefs.getString("binding", null) == binding) {
            check(prefs.edit().putString("endpoint", endpoint).commit())
        }
    }

    fun clear(binding: String): Boolean = synchronized(lock) {
        if (prefs.getString("binding", null) != binding) return@synchronized false
        check(prefs.edit().clear().commit())
        true
    }
    fun openIntent(user: String, id: String, path: String): Intent? = synchronized(lock) {
        val binding = prefs.getString("binding", null)
        if (!matches(binding, user) || pushDestination(path) == null) return@synchronized null
        Intent(context, MainActivity::class.java).setAction(AndroidPushRegistration.OPEN_ACTION)
            .setData(Uri.parse("zoo-push://$binding/$id"))
            .putExtra("binding", binding).putExtra("path", path).putExtra("tag", notificationTag(user, id))
            .addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
    }

    fun forgetAll() = synchronized(lock) {
        prefs.getString("user", null)?.let { clearNotifications(context, it) }
        check(prefs.edit().clear().commit())
    }

    companion object { val lock = Any() }
}

internal suspend fun <T> Task<T>.awaitPushTask(): T = suspendCancellableCoroutine { continuation ->
    addOnSuccessListener { if (continuation.isActive) continuation.resume(it) }
    addOnFailureListener { if (continuation.isActive) continuation.resumeWithException(it) }
    addOnCanceledListener { continuation.cancel() }
}

/** Reconcile on login, permission change, token refresh, connectivity recovery and resume.
 * One conflated worker uses the existing authenticated API; there is no additional polling.
 */
internal class AndroidPushRegistration(private val context: Context, private val app: Session) {
    private val store = PushBinding(context)
    private val binding = store.bind(app.repo.config.apiBaseUrl, app.repo.config.userId)
    private val updates = Channel<Unit>(Channel.CONFLATED)
    private var closed = false
    private var projectId: String? = null
    private var registeredEndpoint: String? = null
    private var retiredEndpoint: String? = null
    private val observer = app.scope.launch {
        snapshotFlow { app.env.online to app.env.visible }.collectLatest { (online, _) -> if (online) refresh() }
    }
    private val worker = app.scope.launch {
        for (ignored in updates) {
            try {
                if (!app.env.online || !notificationsAllowed(context)) continue
                val firebase = FirebaseApp.initializeApp(context) ?: continue
                if (projectId == null) {
                    val config = app.api.request("/api/push/config") as? Row ?: continue
                    projectId = config.text("fcmProjectId").takeIf { it.isNotEmpty() }
                }
                if (projectId == null || projectId != firebase.options.projectId) continue
                val messaging = FirebaseMessaging.getInstance()
                messaging.isAutoInitEnabled = true
                val token = messaging.token.awaitPushTask()
                val endpoint = "fcm:$token:$binding"
                if (endpoint != registeredEndpoint) {
                    val previous = store.endpoint(binding)
                    app.api.request("/api/push/subscriptions", "POST", obj("transport" to "fcm", "endpoint" to endpoint))
                    if (closed) continue
                    store.registered(binding, endpoint)
                    registeredEndpoint = endpoint
                    if (previous != null && previous != endpoint) retiredEndpoint = previous
                }
                retiredEndpoint?.let { previous ->
                    app.api.request("/api/push/subscriptions", "DELETE", obj("endpoint" to previous))
                    retiredEndpoint = null
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                // Retry on the next lifecycle/network/token event; never log registration tokens.
                android.util.Log.w("ZooPush", "Android push registration failed")
            }
        }
    }

    init { active = this; refresh() }
    fun refresh() { if (!closed) updates.trySend(Unit) }

    suspend fun logout() {
        val endpoint = store.endpoint(binding)
        forget() // Reject in-flight pushes before attempting any network cleanup.
        withTimeoutOrNull(5_000) {
            try {
                if (endpoint != null) app.api.request("/api/push/subscriptions", "DELETE", obj("endpoint" to endpoint))
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { /* A delayed delivery is rejected by the cleared binding. */ }
        }
    }

    fun forget() {
        synchronized(PushBinding.lock) {
            if (store.clear(binding)) clearNotifications(context, app.repo.config.userId)
        }
        close()
    }

    fun open(intent: Intent): Boolean {
        if (intent.action != OPEN_ACTION || !store.matches(intent.getStringExtra("binding"), app.repo.config.userId)) return false
        val path = pushDestination(intent.getStringExtra("path")) ?: return false
        app.navigate(path)
        intent.getStringExtra("tag")?.let { context.getSystemService(NotificationManager::class.java).cancel(it, 0) }
        return true
    }

    fun close() {
        closed = true
        observer.cancel()
        worker.cancel()
        updates.close()
        if (active === this) active = null
    }

    companion object {
        const val OPEN_ACTION = "zoo.push.OPEN"
        private var active: AndroidPushRegistration? = null
        fun tokenChanged() { active?.refresh() }
    }
}

internal fun notificationsAllowed(context: Context): Boolean =
    context.getSystemService(NotificationManager::class.java).areNotificationsEnabled() &&
        (Build.VERSION.SDK_INT < 33 || context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED)

internal fun pushDestination(path: String?): String? {
    if (path == null) return null
    val uuid = "[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}"
    return path.takeIf { Regex("/channels/$uuid\\?message=$uuid").matches(it) }
}

internal fun clearNotifications(context: Context, user: String) {
    val manager = context.getSystemService(NotificationManager::class.java)
    manager.activeNotifications.filter { it.tag?.startsWith("zoo:$user:") == true }
        .forEach { manager.cancel(it.tag, it.id) }
}

/** Data-only delivery always passes through the same account/permission checks, including cold starts. */
class ZooMessagingService : FirebaseMessagingService() {
    override fun onNewToken(token: String) { AndroidPushRegistration.tokenChanged() }
    override fun onMessageReceived(message: RemoteMessage) { showPushNotification(this, message.data) }
}

internal fun showPushNotification(context: Context, data: Map<String, String>) {
    synchronized(PushBinding.lock) { showBoundPushNotification(context, data) }
}

private fun showBoundPushNotification(context: Context, data: Map<String, String>) {
    val binding = data["binding"]
    val user = data["userId"]
    if (!PushBinding(context).matches(binding, user) || !notificationsAllowed(context)) return
    val path = pushDestination(data["url"]) ?: return
    val kind = data["notificationType"]
    if (kind !in setOf("mention", "dm", "reply", "reaction")) return
    val id = data["tag"]?.removePrefix("zoo-")?.takeIf { runCatching { UUID.fromString(it) }.isSuccess } ?: return
    val tag = notificationTag(user!!, id)
    val channel = when (kind) { "reaction" -> "reactions"; "dm" -> "messages"; else -> "mentions" }
    val manager = context.getSystemService(NotificationManager::class.java)
    // Existing localized channel names are preserved; cold installs get a safe app-name fallback.
    if (manager.getNotificationChannel("zoo.$channel") == null) {
        manager.createNotificationChannel(NotificationChannel("zoo.$channel", "Zoo",
            if (channel == "reactions") NotificationManager.IMPORTANCE_DEFAULT else NotificationManager.IMPORTANCE_HIGH))
    }
    val intent = PushBinding(context).openIntent(user, id, path) ?: return
    val body = data["body"].orEmpty().take(2000)
    val notification = Notification.Builder(context, "zoo.$channel")
        .setSmallIcon(works.zli.zoo.R.drawable.ic_notification)
        .setContentTitle(data["title"].orEmpty().take(200)).setContentText(body)
        .setStyle(Notification.BigTextStyle().bigText(body))
        .setContentIntent(PendingIntent.getActivity(context, 0, intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
        .setAutoCancel(true).setOnlyAlertOnce(true).setVisibility(Notification.VISIBILITY_PRIVATE)
        .setCategory(Notification.CATEGORY_MESSAGE).build()
    try { manager.notify(tag, 0, notification) }
    catch (_: SecurityException) { /* Permission can be revoked between the check and delivery. */ }
}
