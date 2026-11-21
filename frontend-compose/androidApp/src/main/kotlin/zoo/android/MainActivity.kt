package zoo.android

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.enableEdgeToEdge
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import io.ktor.client.*
import io.ktor.client.engine.okhttp.*
import io.ktor.client.plugins.*
import io.ktor.client.request.*
import io.ktor.client.statement.*
import io.ktor.http.*
import kotlinx.coroutines.*
import kotlinx.serialization.json.*
import zoo.*
import zoo.phone.LocalPhoneController
import zoo.phone.PhoneCallBanner
import zoo.android.phone.AndroidPhoneHost
import zoo.android.phone.PhonePermissionRequest
import zoo.android.update.UpdateSettingsContent
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

    private var notificationIntent by mutableStateOf<Intent?>(null)
    private var notificationPermissionRevision by mutableStateOf(0)
    private val notificationPermission =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) {
            notificationPermissionRevision++
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

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        notificationIntent = intent
        if (UpdatePreferences(this).enabled) UpdateScheduler.configure(this, true)
        setContent { AndroidApp() }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        // 初回ログインと再認証で同じ App Link を使い、待機中の state と合うものだけを受け取る。
        if (intent.action == ReauthCallbackActivity.ACTION_REAUTH_CALLBACK) {
            intent.dataString?.let { reauth?.deliver(it) }
        } else notificationIntent = intent
    }

    /** リカバリーキーをクリップボードに置く。キーボードの候補やプレビューに出ないよう、機微な内容の印を付ける。 */
    private fun copySecret(value: String) {
        val clip = android.content.ClipData.newPlainText("Zoo", value)
        clip.description.extras =
            android.os.PersistableBundle().apply {
                putBoolean(
                    if (android.os.Build.VERSION.SDK_INT >= 33) android.content.ClipDescription.EXTRA_IS_SENSITIVE
                    else "android.content.extra.IS_SENSITIVE",
                    true,
                )
            }
        getSystemService(android.content.ClipboardManager::class.java).setPrimaryClip(clip)
    }

    @Composable
    private fun AndroidApp() {
        var active by remember { mutableStateOf<RunningSession?>(null) }
        var loggingOut by remember { mutableStateOf(false) }
        val dark = isSystemInDarkTheme()
        val loginStore = remember { AndroidLoginStore(this) }
        val scope = rememberCoroutineScope()
        var restoring by remember { mutableStateOf(loginStore.hasSession) }
        var restoreCancelRequested by remember { mutableStateOf(false) }
        var restoreAttempt by remember { mutableIntStateOf(0) }
        var restoreJob by remember { mutableStateOf<Job?>(null) }
        var canRestore by remember { mutableStateOf(false) }
        var restoreError by remember { mutableStateOf("") }
        val current = active
        LaunchedEffect(current) {
            if (current != null && android.os.Build.VERSION.SDK_INT >= 33 &&
                !current.notifications.allowed() &&
                !getSharedPreferences("notification-permission", MODE_PRIVATE).getBoolean("requested", false)) {
                enableNotifications()
            }
        }
        LaunchedEffect(current, notificationIntent) {
            if (current != null) {
                notificationIntent?.let { if (!current.phone.open(it) && !current.push.open(it, current.notifications::dismissTag)) current.notifications.open(it) }
                notificationIntent = null
            }
        }
        DisposableEffect(Unit) { onDispose { active?.close() } }

        suspend fun connect(connection: LoginConnection, refresh: String?) {
            active = openSession(
                connection, loginStore, dark, refresh,
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
            restoring = loginStore.hasSession
            restoreCancelRequested = false
            restoreError = ""
            try {
                val saved = loginStore.load()
                canRestore = saved != null
                if (saved == null) PushBinding(this@MainActivity).forgetAll()
                if (saved != null) connect(saved.connection, saved.refreshToken)
            } catch (failure: CancellationException) {
                throw failure
            } catch (failure: LoginFailure) {
                canRestore = loginStore.hasSession
                restoreError = failure.message.orEmpty()
            } catch (_: Exception) {
                canRestore = loginStore.hasSession
                restoreError = "自動ログインできませんでした。ネットワークを確認して再試行してください。"
            } finally {
                canRestore = loginStore.hasSession
                restoring = false
                restoreJob = null
            }
        }
        if (current == null) {
            CompositionLocalProvider(LocalTouchUi provides true) {
                ZooTheme(dark = dark) {
                    Login(
                        initialError = restoreError,
                        retryRestore = if (canRestore) ({ restoreAttempt++ }) else null,
                        restoring = restoring,
                        cancelRestore = if (restoreCancelRequested) null else ({
                            restoreCancelRequested = true
                            restoreJob?.cancel()
                        }),
                    ) { base, issuer, client, resource ->
                        canRestore = false
                        loginStore.clear()
                        PushBinding(this@MainActivity).forgetAll()
                        connect(LoginConnection(base, issuer, client, resource), null)
                    }
                }
            }
        } else {
            val platform = current.platform
            LaunchedEffect(platform.env, dark) {
                platform.systemDark(dark)
                current.session.update(platform.env)
            }
            DisposableEffect(current) {
                var disposed = false
                val observer = LifecycleEventObserver { _, event ->
                    if (event == Lifecycle.Event.ON_RESUME) current.push.refresh()
                    platform.visible(lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED))
                    current.session.update(platform.env)
                }
                lifecycle.addObserver(observer)
                platform.visible(lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED))
                current.session.update(platform.env)
                val connectivity = getSystemService(android.net.ConnectivityManager::class.java)
                fun updateOnline() = runOnUiThread {
                    if (!disposed) platform.online(hasInternet(
                        connectivity.activeNetwork?.let(connectivity::getNetworkCapabilities),
                    ))
                }
                val callback =
                    object : android.net.ConnectivityManager.NetworkCallback() {
                        override fun onAvailable(network: android.net.Network) = updateOnline()

                        override fun onLost(network: android.net.Network) = updateOnline()

                        override fun onCapabilitiesChanged(
                            network: android.net.Network,
                            caps: android.net.NetworkCapabilities,
                        ) = updateOnline()
                    }
                connectivity.registerDefaultNetworkCallback(callback)
                updateOnline()
                onDispose {
                    disposed = true
                    lifecycle.removeObserver(observer)
                    connectivity.unregisterNetworkCallback(callback)
                }
            }
            BackHandler {
                when {
                    current.session.escape() -> Unit
                    current.session.thread != null -> current.session.thread = null
                    platform.back() -> Unit
                    else -> moveTaskToBack(true)
                }
            }
            val encryption =
                remember(current) {
                    current.e2eeSettings?.let { host ->
                        @Composable { app: Session -> zoo.crypto.EncryptionSettings(app, host) }
                    }
                }
            CompositionLocalProvider(
                LocalWebPlatform provides platform,
                LocalApp provides current.session,
                LocalEncryptionSettings provides encryption,
                LocalAndroidUpdateSettings provides { UpdateSettingsContent(this@MainActivity) },
                LocalAndroidNotificationSettings provides {
                    val allowed = remember(notificationPermissionRevision, platform.env.visible) {
                        current.notifications.allowed()
                    }
                    zoo.Text(current.session.t("androidNotifications.scope"))
                    zoo.Button(
                        current.session.t(if (allowed) "settings.navTitle" else "androidNotifications.enable"),
                        icon = "bell",
                        click = ::enableNotifications,
                    )
                },
                LocalTouchUi provides true,
                LocalPhoneController provides current.phone.controller,
            ) {
                ZooTheme(platform.env.dark, locale = platform.env.locale) {
                    Column(Modifier.fillMaxSize().safeDrawingPadding().imePadding().operationHaptics(platform.env.operationHaptics)) {
                        Box(Modifier.weight(1f)) {
                            NativeWindow(current.session) { ZooApp(current.session) }
                        }
                        PhoneCallBanner(current.session, current.phone.controller)
                    }
                    unbackedKeysPrompt?.let { UnbackedKeysDialog(platform, it) }
                }
            }
        }
    }

    private suspend fun openSession(
        connection: LoginConnection,
        loginStore: AndroidLoginStore,
        dark: Boolean,
        savedRefresh: String?,
        onExpired: (DeviceAuth) -> Unit,
        logout: () -> Unit,
    ): RunningSession {
        connection.validate()
        val owner = loginStore.beginSession(savedRefresh?.let { SavedLogin(connection, it) })
        val http = HttpClient(OkHttp) {
            // Refresh tokens are single-use: never replay a request inside OkHttp.
            engine {
                config {
                    retryOnConnectionFailure(false)
                    followRedirects(false)
                    followSslRedirects(false)
                }
            }
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
        )
        val reauthenticator = AndroidReauth(this, http, connection)
        reauth = reauthenticator
        try {
            if (savedRefresh == null)
                auth.acceptBrowserLogin(connection, reauthenticator.signIn())
            else auth.restore(connection.issuer, connection.client, connection.resource, savedRefresh)
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
            val lifetime = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
            val platform = AndroidPlatform(
                this, http, auth, connection.base, me.text("id"), logout, ::chooseFiles, recentPhotos, ::requestRecentPhotos, ::confirmUnbackedKeys,
            )
            platform.systemDark(dark)
            platform.visible(lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED))
            val config = Config(
                connection.base, "/api/shapes", me.text("id"), me.flag("is_admin"),
                assetsBaseUrl = connection.base,
            )
            val session = Session(Repository(config, platform, http, lifetime), lifetime, platform.env)
            // ログインしたら、E2EE の端末を登録する（#359）。失敗は表示せず、裏で試し直す。
            val e2eeApi = zoo.crypto.ZooE2eeApi(ZooApi(config, platform, http))
            val e2ee = zoo.crypto.E2eeDeviceManager(
                api = e2eeApi,
                server = connection.base,
                userId = me.text("id"),
                displayName = e2eeDisplayName(),
                files = zoo.crypto.CryptoStoreFiles.android(this, connection.base, me.text("id")),
                pending = zoo.crypto.PendingDeviceDeletions.android(this),
                toDevice = { deviceId -> zoo.crypto.toDeviceShape(config, platform, http, deviceId) },
                scope = lifetime,
                onError = { failure ->
                    android.util.Log.w("ZooE2ee", "E2EE の端末の処理に失敗しました: ${failure::class.simpleName}: ${failure.message}")
                },
                // 初回設定と解錠（#360）。解錠済みの端末は、起動時の確認と鍵バックアップへの送信も行う。
                setupApi = e2eeApi,
            )
            platform.e2ee = e2ee
            platform.messageEncryption = zoo.crypto.AndroidMessageEncryption(e2ee, session.api, lifetime)
            // 設定の「暗号化」タブ（#360）。再認証は Custom Tabs、パスキーは Credential Manager。
            val e2eeSettings = e2ee.setup?.let { setup ->
                zoo.crypto.E2eeSettingsHost(
                    setup = setup,
                    rpId = passkeyRpId(connection.issuer),
                    passkeys = AndroidPasskeys(this),
                    reauthenticate = reauthenticator::reauthenticate,
                    copySecret = ::copySecret,
                )
            }
            val logoutApi = ZooApi(config, object : Platform by platform {
                override suspend fun headers() = auth.headersWithoutRefresh()
            }, http)
            return RunningSession(session, platform, lifetime, http, auth, e2ee, e2eeSettings, AndroidNotifications(this, session), AndroidPushRegistration(this, session, logoutApi), AndroidPhoneHost(applicationContext, session, http, ::preparePhone)).also {
                session.start()
                e2ee.start()
            }
        } catch (failure: Throwable) {
            if (reauth === reauthenticator) reauth = null
            auth.close()
            http.close()
            throw failure
        }
    }

    @Composable
    private fun Login(
        initialError: String = "",
        retryRestore: (() -> Unit)? = null,
        restoring: Boolean = false,
        cancelRestore: (() -> Unit)? = null,
        connect: suspend (String, String, String, String) -> Unit
    ) {
        val prefs = remember { getSharedPreferences("connection", 0) }
        var base by rememberSaveable {
            mutableStateOf(prefs.getString("base", "https://zli.chat").orEmpty())
        }
        var issuer by rememberSaveable {
            mutableStateOf(
                prefs.getString("issuer", "https://account.zli.works/api/auth").orEmpty()
            )
        }
        var client by rememberSaveable { mutableStateOf(prefs.getString("client", "").orEmpty()) }
        var resource by rememberSaveable {
            mutableStateOf(prefs.getString("resource", "https://zli.chat/api").orEmpty())
        }
        var advanced by rememberSaveable { mutableStateOf(false) }
        var error by remember(initialError) { mutableStateOf(initialError) }
        var job by remember { mutableStateOf<Job?>(null) }
        val scope = rememberCoroutineScope()
        AndroidSignInScreen(
            message = if (restoring) "" else "ブラウザーで認証を完了すると、Zooに戻ります。",
            detailsVisible = advanced && !restoring,
            busy = restoring || job != null,
            details = {
                if (advanced && !restoring) {
                    Field(base, "Zoo サーバー URL", Modifier.fillMaxWidth(), enabled = job == null,
                        inputAttributes = mapOf("type" to "url")) { base = it }
                    Field(issuer, "OIDC issuer", Modifier.fillMaxWidth(), enabled = job == null,
                        inputAttributes = mapOf("type" to "url")) { issuer = it }
                    Field(client, "Android 用 public client ID", Modifier.fillMaxWidth(), enabled = job == null) {
                        client = it
                    }
                    Field(resource, "Resource / audience", Modifier.fillMaxWidth(), enabled = job == null,
                        inputAttributes = mapOf("type" to "url")) { resource = it }
                }
                if (!restoring && error.isNotEmpty()) Text(
                    error,
                    modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
                    color = MaterialTheme.colorScheme.error,
                )
            },
            actions = {
                when {
                    restoring -> zoo.Button("ログイン画面を開く", enabled = cancelRestore != null, variant = "ghost") {
                        cancelRestore?.invoke()
                    }
                    job != null -> zoo.Button("キャンセル", variant = "ghost") { job?.cancel() }
                    else -> {
                        if (client.isNotBlank() || advanced) {
                            zoo.Button(if (advanced) "接続先の設定を閉じる" else "接続先の設定", variant = "ghost") {
                                advanced = !advanced
                            }
                        }
                        if (retryRestore != null) {
                            zoo.Button("自動ログインを再試行", variant = "ghost", click = retryRestore)
                        }
                        zoo.Button(
                            if (client.isBlank() && !advanced) "接続先を設定" else "ログイン",
                            enabled = client.isNotBlank() || !advanced,
                            primary = true,
                            contentInset = 24.dp,
                        ) {
                            if (client.isBlank()) {
                                advanced = true
                                return@Button
                            }
                            job =
                                scope.launch {
                                    error = ""
                                    try {
                                        val server = base.trim().trimEnd('/')
                                        val url = Url(server)
                                        require(
                                            url.protocol == URLProtocol.HTTPS &&
                                                url.user.isNullOrEmpty() &&
                                                url.password.isNullOrEmpty() &&
                                                url.encodedPath in listOf("", "/") &&
                                                url.parameters.isEmpty() &&
                                                url.fragment.isEmpty()
                                        ) {
                                            "サーバーには HTTPS の origin URL を指定してください"
                                        }
                                        prefs
                                            .edit()
                                            .putString("base", server)
                                            .putString("issuer", issuer.trim())
                                            .putString("client", client.trim())
                                            .putString("resource", resource.trim())
                                            .apply()
                                        connect(
                                            server,
                                            issuer.trim(),
                                            client.trim(),
                                            resource.trim(),
                                        )
                                    } catch (e: CancellationException) {
                                        throw e
                                    } catch (failure: LoginFailure) {
                                        error = failure.message.orEmpty()
                                    } catch (failure: zoo.crypto.ReauthException) {
                                        error = failure.message.orEmpty()
                                    } catch (_: Exception) {
                                        error = "ログインできません。接続先とネットワークを確認してください。"
                                    } finally {
                                        job = null
                                    }
                                }
                        }
                    }
                }
            },
        )
    }
}

private class RunningSession(
    val session: Session,
    val platform: AndroidPlatform,
    val scope: CoroutineScope,
    private val http: HttpClient,
    val auth: DeviceAuth,
    private val e2ee: zoo.crypto.E2eeDeviceManager,
    val e2eeSettings: zoo.crypto.E2eeSettingsHost?,
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
    }
}

/**
 * パスキーの RP ID（docs/e2ee.md の「RP ID の決め方」）。ログインに使う issuer のホスト名（本番は
 * `account.zli.works`）。HTTPS でなければ `null`（リカバリーキーだけを使う）。
 */
private fun passkeyRpId(issuer: String): String? =
    try {
        Url(issuer).takeIf { it.protocol == URLProtocol.HTTPS }?.host?.takeIf { it.isNotEmpty() }
    } catch (_: Exception) {
        null
    }

/** 端末名（`display_name`）。本人の端末一覧（#362）にだけ出る。例：`Zoo Android（Pixel 8）`。 */
private fun e2eeDisplayName(): String =
    "Zoo Android（${android.os.Build.MODEL.orEmpty().ifBlank { "Android" }.take(100)}）"

/** 鍵バックアップに送れていない鍵があるときの、ログアウトの確認（React の `UnbackedKeysLogoutDialog`）。 */
@Composable
private fun UnbackedKeysDialog(platform: AndroidPlatform, prompt: UnbackedKeysPrompt) {
    val t = { key: String -> platform.translate(key, mapOf("count" to prompt.count.toString())) }
    AlertDialog(
        onDismissRequest = { prompt.answer.complete(false) },
        title = { Text(t("e2ee.logout.unbackedTitle")) },
        text = {
            Text(
                when {
                    prompt.count < 0 -> t("e2ee.logout.unknownDescription")
                    prompt.count == 1 -> t("e2ee.logout.unbackedDescription_one")
                    else -> t("e2ee.logout.unbackedDescription_other")
                }
            )
        },
        confirmButton = {
            TextButton(onClick = { prompt.answer.complete(true) }) {
                Text(t("e2ee.logout.confirm"), color = MaterialTheme.colorScheme.error)
            }
        },
        dismissButton = {
            TextButton(onClick = { prompt.answer.complete(false) }) { Text(t("common.cancel")) }
        },
    )
}
