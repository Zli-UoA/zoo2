package zoo.android

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.enableEdgeToEdge
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.browser.auth.AuthTabIntent
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.*
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import io.ktor.client.*
import io.ktor.client.plugins.*
import io.ktor.client.request.*
import io.ktor.client.statement.*
import io.ktor.http.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.serialization.json.*
import zoo.*
import zoo.android.phone.AndroidPhoneHost
import zoo.android.phone.PhonePermissionRequest
import zoo.android.phone.PhonePictureInPictureActivity
import zoo.android.ui.DialogAction
import zoo.android.ui.UiIcon
import zoo.android.update.UpdatePreferences
import zoo.android.update.UpdateScheduler

/** 鍵バックアップに送れていない鍵があるときの、ログアウトの確認（#359）。 */
private class UnbackedKeysPrompt(val count: Int, val answer: CompletableDeferred<Boolean>)

class MainActivity : ComponentActivity() {
    private var unbackedKeysPrompt by mutableStateOf<UnbackedKeysPrompt?>(null)

    private suspend fun confirmUnbackedKeys(count: Int): Boolean {
        val answer = CompletableDeferred<Boolean>()
        unbackedKeysPrompt = UnbackedKeysPrompt(count, answer)
        return try {
            answer.await()
        } finally {
            unbackedKeysPrompt = null
        }
    }

    private var notificationSettingsVersion by mutableIntStateOf(0)
    private var shared by mutableStateOf<IncomingShare?>(null)
    private var notificationIntent by mutableStateOf<Intent?>(null)
    private val notificationPermission =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) {
            notificationSettingsVersion++
            AndroidPushRegistration.tokenChanged()
        }

    private fun enableNotifications() {
        val prefs = getSharedPreferences("notification-permission", MODE_PRIVATE)
        val requested = prefs.getBoolean("requested", false)
        if (android.os.Build.VERSION.SDK_INT >= 33 &&
            checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED &&
            (!requested || shouldShowRequestPermissionRationale(android.Manifest.permission.POST_NOTIFICATIONS))) {
            prefs.edit().putBoolean("requested", true).apply()
            notificationPermission.launch(android.Manifest.permission.POST_NOTIFICATIONS)
        } else {
            startActivity(Intent(android.provider.Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                .putExtra(android.provider.Settings.EXTRA_APP_PACKAGE, packageName))
        }
    }

    private fun openIncomingSettings() {
        try {
            startActivity(Intent(android.provider.Settings.ACTION_MANAGE_APP_USE_FULL_SCREEN_INTENT,
                android.net.Uri.fromParts("package", packageName, null)))
        } catch (_: android.content.ActivityNotFoundException) {
            startActivity(Intent(android.provider.Settings.ACTION_APP_NOTIFICATION_SETTINGS)
                .putExtra(android.provider.Settings.EXTRA_APP_PACKAGE, packageName))
        }
    }

    private val phonePermissionRequest = PhonePermissionRequest()
    private val phonePermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted -> phonePermissionRequest.complete(granted) }
    private suspend fun preparePhone() {
        if (checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) == android.content.pm.PackageManager.PERMISSION_GRANTED) return
        check(lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED))
        phonePermissionRequest.await { phonePermission.launch(android.Manifest.permission.RECORD_AUDIO) }
    }

    private val recentPhotos by lazy { RecentPhotos(this) }
    private var photoPermissionResult: CompletableDeferred<Unit>? = null
    private val photoPermissions = registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
        photoPermissionResult?.complete(Unit)
        photoPermissionResult = null
    }

    private suspend fun requestRecentPhotos() {
        if (photoPermissionResult != null || recentPhotos.permissions().isEmpty()) return
        val result = CompletableDeferred<Unit>()
        photoPermissionResult = result
        try {
            photoPermissions.launch(recentPhotos.permissions())
            result.await()
        } finally {
            if (photoPermissionResult === result) photoPermissionResult = null
        }
    }

    private var saveResult: CompletableDeferred<Uri?>? = null
    private val savePicker = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) {
        saveResult?.complete(it.data?.data.takeIf { _ -> it.resultCode == RESULT_OK })
        saveResult = null
    }
    private suspend fun saveFile(bytes: ByteArray, name: String, type: String) {
        check(saveResult == null)
        val answer = CompletableDeferred<Uri?>()
        saveResult = answer
        val destination = try {
            savePicker.launch(Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
                .setType(type.ifBlank { "application/octet-stream" }).putExtra(Intent.EXTRA_TITLE, name))
            answer.await()
        } finally { if (saveResult === answer) saveResult = null }
        if (destination != null) withContext(Dispatchers.IO) {
            checkNotNull(contentResolver.openOutputStream(destination, "wt")).use { it.write(bytes) }
        }
    }

    private var fileResult: CompletableDeferred<List<Uri>>? = null
    private val picker =
        registerForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) {
            fileResult?.complete(it)
            fileResult = null
        }

    private val photoPicker =
        registerForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(10)) {
            fileResult?.complete(it)
            fileResult = null
        }

    private suspend fun chooseFiles(photos: Boolean): List<Uri> {
        check(fileResult == null) { "ファイル選択中です" }
        val result = CompletableDeferred<List<Uri>>()
        fileResult = result
        return try {
            if (photos) photoPicker.launch(
                androidx.activity.result.PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly),
            ) else picker.launch(arrayOf("*/*"))
            result.await()
        } finally {
            if (fileResult === result) fileResult = null
        }
    }

    /** 今のログインの E2EE の再認証（#360）。App Links で戻ったリダイレクトを渡す。 */
    @Volatile
    private var reauth: AndroidReauth? = null

    private var authTabResult: ((AuthTabIntent.AuthResult) -> Unit)? = null
    private val authTab = AuthTabIntent.registerActivityResultLauncher(this) { result ->
        val receive = authTabResult
        authTabResult = null
        receive?.invoke(result)
    }

    private fun launchAuthTab(
        tab: AuthTabIntent, url: Uri, redirect: Uri, receive: (AuthTabIntent.AuthResult) -> Unit,
    ): Boolean {
        // A canceled app coroutine can leave a browser activity open. Its result belongs to that launch.
        if (authTabResult != null) return false
        authTabResult = receive
        try {
            tab.launch(authTab, url, requireNotNull(redirect.host), requireNotNull(redirect.path))
        } catch (failure: RuntimeException) {
            authTabResult = null
            throw failure
        }
        return true
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        receiveLaunchIntent(intent)
        setContent { AndroidApp() }
        // A first draw is queued before any update bookkeeping opens disk-backed stores.
        window.decorView.postOnAnimation {
            window.decorView.post {
                val updates = UpdatePreferences(this)
                updates.confirmInstalledUpdate(works.zli.zoo.BuildConfig.VERSION_CODE.toLong(), works.zli.zoo.BuildConfig.VERSION_NAME)
                if (updates.enabled) UpdateScheduler.configure(this, true)
            }
        }
    }

    private fun receiveLaunchIntent(intent: Intent) {
        if (intent.action == Intent.ACTION_SEND || intent.action == Intent.ACTION_SEND_MULTIPLE) {
            shared = incomingShare(intent)
            if (shared == null) android.widget.Toast.makeText(this, "共有内容を開けません", android.widget.Toast.LENGTH_SHORT).show()
        } else notificationIntent = intent
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        // 初回ログインと再認証で同じ App Link を使い、待機中の state と合うものだけを受け取る。
        if (intent.action == ReauthCallbackActivity.ACTION_REAUTH_CALLBACK) {
            intent.dataString?.let { reauth?.deliver(it) }
        } else receiveLaunchIntent(intent)
    }

    @Composable
    private fun AndroidApp() {
        var active by remember { mutableStateOf<RunningSession?>(null) }
        var loggingOut by remember { mutableStateOf(false) }
        val dark = isSystemInDarkTheme()
        val loginStore = remember { AndroidLoginStore(this) }
        val scope = rememberCoroutineScope()
        var restoring by remember { mutableStateOf(true) }
        var restorationStage by remember { mutableStateOf(LoginStage.SavedSession) }
        var restoreCancelRequested by remember { mutableStateOf(false) }
        var restoreAttempt by remember { mutableIntStateOf(0) }
        var restoreJob by remember { mutableStateOf<Job?>(null) }
        var canRestore by remember { mutableStateOf(false) }
        var restoreError by remember { mutableStateOf("") }
        var restoreDiagnostics by remember { mutableStateOf("") }
        val current = active
        val lightIconsBackground = !(current?.platform?.env?.dark ?: dark)
        SideEffect {
            val controller = androidx.core.view.WindowCompat.getInsetsController(window, window.decorView)
            controller.isAppearanceLightStatusBars = lightIconsBackground
            controller.isAppearanceLightNavigationBars = lightIconsBackground
        }
        var callPresentation by remember(current) { mutableIntStateOf(0) }
        LaunchedEffect(current) {
            if (current != null && android.os.Build.VERSION.SDK_INT >= 33 &&
                !current.notifications.allowed() &&
                !getSharedPreferences("notification-permission", MODE_PRIVATE).getBoolean("requested", false)) {
                enableNotifications()
            }
        }
        LaunchedEffect(current, notificationIntent) {
            if (current != null) {
                notificationIntent?.let { intent ->
                    if (!current.phone.open(intent) { callPresentation++ } &&
                        !current.push.open(intent, current.notifications::dismissTag)
                    ) {
                        current.notifications.open(intent)
                    }
                }
                notificationIntent = null
            }
        }
        DisposableEffect(Unit) { onDispose { active?.close() } }

        suspend fun connect(connection: LoginConnection, refresh: String?, stage: (LoginStage) -> Unit) {
            active = openSession(
                connection, loginStore, dark, refresh, stage,
                onExpired = { expiredAuth ->
                    scope.launch {
                        val running = active
                        if (running == null && !loginStore.hasSession) PushBinding(this@MainActivity).forgetAll()
                        if (running != null && running.auth === expiredAuth) {
                            active = null
                            canRestore = false
                            restoreError = "認証情報の有効期限が切れました。ログインし直してください。"
                            running.notifications.close()
                            running.push.forget()
                            running.close(preserveNotifications = false)
                        }
                    }
                },
                logout = {
                    if (!loggingOut) {
                        loggingOut = true
                        scope.launch {
                            val running = active
                            try {
                                running?.logout()
                                restoreError = ""
                            } catch (failure: LoginFailure) {
                                restoreError = failure.message.orEmpty()
                            } finally {
                                active = null
                                canRestore = false
                                loggingOut = false
                            }
                        }
                    }
                },
            )
        }

        LaunchedEffect(restoreAttempt) {
            restoreJob = currentCoroutineContext()[Job]
            restoring = true
            restorationStage = LoginStage.SavedSession
            restoreCancelRequested = false
            restoreError = ""
            restoreDiagnostics = ""
            val stages = mutableListOf(LoginStage.SavedSession)
            try {
                val saved = withContext(Dispatchers.IO) { loginStore.load() }
                canRestore = saved != null
                if (saved == null) PushBinding(this@MainActivity).forgetAll()
                if (saved != null) connect(saved.connection, saved.refreshToken) { stage -> stages.add(stage); restorationStage = stage }
            } catch (failure: CancellationException) {
                throw failure
            } catch (failure: LoginFailure) {
                canRestore = loginStore.hasSession
                restoreError = failure.message.orEmpty()
                restoreDiagnostics = loginDiagnosticLog(stages, failure)
            } catch (failure: Exception) {
                canRestore = loginStore.hasSession
                restoreError = loginNetworkMessage(failure, captivePortal = false)
                    ?: "自動ログインできませんでした。ネットワークを確認して再試行してください。"
                restoreDiagnostics = loginDiagnosticLog(stages, failure)
            } finally {
                canRestore = loginStore.hasSession
                restoring = false
                restoreJob = null
            }
        }
        if (current == null) {
            SignInRoute(dark, restoring, restoreError, restorationStage = restorationStage,
                retry = if (canRestore && !restoring) ({ restoreAttempt++ }) else null,
                cancelRestore = if (restoreCancelRequested) null else ({
                    restoreCancelRequested = true
                    restoreJob?.cancel()
                }),
                restoreDiagnostics = restoreDiagnostics,
                connect = { connection, stage ->
                    restoreDiagnostics = ""
                    canRestore = false
                    loginStore.clear()
                    PushBinding(this@MainActivity).forgetAll()
                    connect(connection, null, stage)
                },
            )
        } else {
            val platform = current.platform
            LaunchedEffect(platform.env, dark) {
                platform.systemDark(dark)
                current.session.update(platform.env)
            }
            DisposableEffect(current) {
                var disposed = false
                val connectivity = getSystemService(android.net.ConnectivityManager::class.java)
                var network = connectivity.activeInternetNetwork()
                fun updateNetworkStatus(returning: Boolean = false) {
                    val nextNetwork = connectivity.activeInternetNetwork()
                    val networkChanged = nextNetwork != null && network != nextNetwork
                    val wasOnline = platform.env.online
                    network = nextNetwork
                    if (returning || networkChanged) current.transport.reconnect()
                    platform.online(nextNetwork != null)
                    current.session.update(platform.env, networkChanged = networkChanged)
                    if (nextNetwork != null && (returning || networkChanged || !wasOnline)) {
                        current.shapeReconnects.update { it + 1 }
                        current.phone.refreshInvites()
                    }
                }
                val observer = LifecycleEventObserver { _, event ->
                    val returning = event == Lifecycle.Event.ON_START && !platform.env.visible
                    platform.visible(lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED))
                    updateNetworkStatus(returning)
                    if (event == Lifecycle.Event.ON_RESUME) { notificationSettingsVersion++; current.push.refresh() }
                }
                lifecycle.addObserver(observer)
                val callback = DefaultNetworkCallback {
                    runOnUiThread {
                        if (!disposed) updateNetworkStatus()
                    }
                }
                connectivity.registerDefaultNetworkCallback(callback)
                updateNetworkStatus()
                val offlineNetworkCheck = scope.launch {
                    recheckDefaultNetworkWhileOffline(snapshotFlow { platform.env.visible && !platform.env.online }) {
                        updateNetworkStatus()
                    }
                }
                onDispose {
                    disposed = true
                    offlineNetworkCheck.cancel()
                    lifecycle.removeObserver(observer)
                    connectivity.unregisterNetworkCallback(callback)
                }
            }
            ConnectedScreens(current.session, platform, current.phone.controller,
                settingsVersion = notificationSettingsVersion,
                incoming = shared,
                finishShare = {
                    shared = null
                    if (intent.action in setOf(Intent.ACTION_SEND, Intent.ACTION_SEND_MULTIPLE)) setIntent(Intent(Intent.ACTION_MAIN))
                },
                enableNotifications = ::enableNotifications,
                background = { moveTaskToBack(true) },
                callPresentation = callPresentation,
                openIncomingSettings = if (android.os.Build.VERSION.SDK_INT >= 34) ::openIncomingSettings else null,
                openPictureInPicture = if (packageManager.hasSystemFeature(
                    android.content.pm.PackageManager.FEATURE_PICTURE_IN_PICTURE,
                )) { id -> PhonePictureInPictureActivity.open(this, current.phone, id) } else null,
            )
            unbackedKeysPrompt?.let { prompt ->
                val args = "count" to prompt.count.toString()
                val detail = when {
                    prompt.count == REVOCATION_PENDING_PROMPT -> current.session.t("e2ee.logout.revocationDescription")
                    prompt.count < 0 -> current.session.t("e2ee.logout.unknownDescription")
                    prompt.count == 1 -> current.session.t("e2ee.logout.unbackedDescription_one", args)
                    else -> current.session.t("e2ee.logout.unbackedDescription_other", args)
                }
                AlertDialog(
                    onDismissRequest = { prompt.answer.complete(false) },
                    title = { Text(current.session.t(if (prompt.count == REVOCATION_PENDING_PROMPT) "e2ee.logout.revocationTitle" else "e2ee.logout.unbackedTitle")) },
                    text = { Text(detail) },
                    confirmButton = {
                        DialogAction(current.session.t("e2ee.logout.confirm"), UiIcon.SignIn) {
                            prompt.answer.complete(true)
                        }
                    },
                    dismissButton = {
                        DialogAction(current.session.t("common.cancel"), UiIcon.Close) {
                            prompt.answer.complete(false)
                        }
                    },
                )
            }
        }
    }

    private suspend fun openSession(
        connection: LoginConnection,
        loginStore: AndroidLoginStore,
        dark: Boolean,
        savedRefresh: String?,
        stage: (LoginStage) -> Unit,
        onExpired: (DeviceAuth) -> Unit,
        logout: () -> Unit,
    ): RunningSession {
        connection.validate()
        val owner = withContext(Dispatchers.IO) { loginStore.beginSession(savedRefresh?.let { SavedLogin(connection, it) }) }
        val oidcRoutes = OidcNetworkRoutes()
        val transport = SessionHttpEngine {
            // Refresh tokens are single-use: never replay a request inside OkHttp.
            config {
                retryOnConnectionFailure(false)
                dns(oidcRoutes)
                followRedirects(false)
                followSslRedirects(false)
            }
        }
        val http = HttpClient(transport) {
            followRedirects = false
            install(HttpTimeout) {
                requestTimeoutMillis = 120_000
                socketTimeoutMillis = 90_000
            }
        }
        val auth = DeviceAuth(
            http,
            persistRefresh = { refresh ->
                if (refresh == null) loginStore.clear(owner)
                else loginStore.save(SavedLogin(connection, refresh), owner)
            },
            consumeRefresh = { loginStore.consume(owner) },
            onExpired = onExpired,
            readDiscovery = oidcRoutes::discover,
        )
        val reauthenticator = AndroidReauth(this, http, connection, ::launchAuthTab, oidcRoutes::discover)
        reauth = reauthenticator
        try {
            if (savedRefresh == null) {
                auth.acceptBrowserLogin(connection, reauthenticator.signIn(stage))
            } else {
                auth.restore(connection.issuer, connection.client, connection.resource, savedRefresh, stage)
            }
            stage(LoginStage.ZooApi)
            val authHeaders = auth.headers()
            val response = http.get(endpoint(connection.base, "/api/users/me")) {
                headers { authHeaders.forEach { (key, value) -> append(key, value) } }
            }
            if (response.status.value !in 200..299) {
                if (response.status.value in setOf(401, 403)) auth.forget()
                throw LoginFailure("Zoo に接続できません (HTTP ${response.status.value})")
            }
            val me = json.parseToJsonElement(response.bodyAsText()).jsonObject
            require(me.text("id").isNotBlank())
            currentCoroutineContext().ensureActive()
            stage(LoginStage.Session)
            val lifetime = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
            val platform = AndroidPlatform(
                this, http, auth, connection.base, me.text("id"), logout, ::chooseFiles, recentPhotos, ::requestRecentPhotos, ::confirmUnbackedKeys, ::saveFile,
            )
            platform.systemDark(dark)
            platform.visible(lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED))
            val config = Config(
                connection.base, "/api/shapes", me.text("id"), me.flag("is_admin"),
                assetsBaseUrl = connection.base,
            )
            val session = Session(Repository(config, platform, http, lifetime), lifetime, platform.env)
            session.setNavigationCacheLimit(platform.navigationCacheLimit)
            lifetime.launch {
                if (!platform.enforceAttachmentCacheSize()) session.toast = session.t("offline.deleteFailed")
            }
            platform.onNavigation = { session.update(it) }
            // ログインしたら、E2EE の端末を登録する（#359）。失敗は表示せず、裏で試し直す。
            val shapeReconnects = MutableStateFlow(0L)
            val e2eeApi = zoo.crypto.ZooE2eeApi(ZooApi(config, platform, http))
            val e2ee = zoo.crypto.E2eeDeviceManager(
                api = e2eeApi,
                server = connection.base,
                userId = me.text("id"),
                displayName = e2eeDisplayName(),
                files = zoo.crypto.CryptoStoreFiles.android(this, connection.base, me.text("id")),
                pending = zoo.crypto.PendingDeviceDeletions.android(this),
                toDevice = { deviceId -> zoo.crypto.toDeviceShape(config, platform, http, deviceId, shapeReconnects) },
                scope = lifetime,
                onError = { failure ->
                    android.util.Log.w("ZooE2ee", "E2EE の端末の処理に失敗しました: ${failure::class.simpleName}: ${failure.message}")
                },
                // 初回設定と解錠（#360）。解錠済みの端末は、起動時の確認と鍵バックアップへの送信も行う。
                setupApi = e2eeApi,
                // 他のユーザーの検証（#368）。to-device で届いた検証のメッセージを受け取る。
                sasApi = e2eeApi,
            )
            platform.e2ee = e2ee
            platform.reauthenticate = reauthenticator::reauthenticate
            platform.issuer = connection.issuer
            runCatching { io.ktor.http.Url(connection.issuer) }.getOrNull()
                ?.takeIf { it.protocol == io.ktor.http.URLProtocol.HTTPS && it.host.isNotBlank() }
                ?.let { platform.passkey = zoo.crypto.UnlockMethod.Passkey(AndroidPasskeys(this), it.host) }
            platform.messageEncryption = zoo.crypto.AndroidMessageEncryption(e2ee, session.api, lifetime)
            val logoutApi = ZooApi(config, object : Platform by platform {
                override suspend fun headers() = auth.headersWithoutRefresh()
            }, http)
            return RunningSession(session, platform, lifetime, http, transport, shapeReconnects, auth, e2ee, AndroidNotifications(this, session), AndroidPushRegistration(this, session, logoutApi), AndroidPhoneHost(applicationContext, session, http, ::preparePhone)).also {
                session.start()
                e2ee.start()
            }
        } catch (failure: Throwable) {
            if (reauth === reauthenticator) reauth = null
            auth.close()
            http.close()
            transport.close()
            throw failure
        }
    }

}

private class RunningSession(
    val session: Session,
    val platform: AndroidPlatform,
    val scope: CoroutineScope,
    private val http: HttpClient,
    val transport: SessionHttpEngine,
    val shapeReconnects: MutableStateFlow<Long>,
    val auth: DeviceAuth,
    private val e2ee: zoo.crypto.E2eeDeviceManager,
    val notifications: AndroidNotifications,
    val push: AndroidPushRegistration,
    val phone: AndroidPhoneHost,
) {
    suspend fun logout() {
        notifications.close()
        try { push.logout() } finally {
            try { auth.forget() } finally { close(preserveNotifications = false) }
        }
    }

    fun close(preserveNotifications: Boolean = true) {
        phone.close()
        e2ee.stop()
        push.close()
        notifications.close(preserveNotifications = preserveNotifications)
        auth.close()
        session.close()
        scope.cancel()
        platform.close()
        http.close()
        transport.close()
    }
}

/** 端末名（`display_name`）。本人の端末一覧（#362）にだけ出る。例：`Zoo Android（Pixel 8）`。 */
private fun e2eeDisplayName(): String =
    "Zoo Android（${android.os.Build.MODEL.orEmpty().ifBlank { "Android" }.take(100)}）"
