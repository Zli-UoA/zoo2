package zoo.android

import android.content.Intent
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
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
import zoo.android.update.AndroidUpdateFrame
import zoo.android.update.UpdatePreferences
import zoo.android.update.UpdateScheduler

class MainActivity : ComponentActivity() {
    private var fileResult: CompletableDeferred<List<Uri>>? = null
    private val picker =
        registerForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) {
            fileResult?.complete(it)
            fileResult = null
        }

    private suspend fun chooseFiles(): List<Uri> {
        check(fileResult == null) { "ファイル選択中です" }
        val result = CompletableDeferred<List<Uri>>()
        fileResult = result
        picker.launch(arrayOf("*/*"))
        return try {
            result.await()
        } finally {
            if (fileResult === result) fileResult = null
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (UpdatePreferences(this).enabled) UpdateScheduler.configure(this, true)
        setContent { AndroidApp() }
    }

    @Composable
    private fun AndroidApp() {
        var active by remember { mutableStateOf<RunningSession?>(null) }
        val dark = isSystemInDarkTheme()
        val loginStore = remember { AndroidLoginStore(this) }
        val scope = rememberCoroutineScope()
        var restoring by remember { mutableStateOf(true) }
        var restoreCancelRequested by remember { mutableStateOf(false) }
        var restoreAttempt by remember { mutableIntStateOf(0) }
        var restoreJob by remember { mutableStateOf<Job?>(null) }
        var canRestore by remember { mutableStateOf(false) }
        var restoreError by remember { mutableStateOf("") }
        val current = active
        DisposableEffect(Unit) { onDispose { active?.close() } }

        suspend fun connect(connection: LoginConnection, refresh: String?, show: (String, String) -> Unit) {
            active = openSession(
                connection, loginStore, dark, refresh, show,
                onExpired = { expiredAuth ->
                    scope.launch {
                        val running = active
                        if (running != null && running.auth === expiredAuth) {
                            active = null
                            canRestore = false
                            restoreError = "認証情報の有効期限が切れました。ログインし直してください。"
                            running.close()
                        }
                    }
                },
                logout = {
                    val running = active
                    active = null
                    canRestore = false
                    restoreError = ""
                    try {
                        running?.logout()
                    } catch (failure: LoginFailure) {
                        restoreError = failure.message.orEmpty()
                    }
                },
            )
        }

        LaunchedEffect(restoreAttempt) {
            restoreJob = currentCoroutineContext()[Job]
            restoring = true
            restoreCancelRequested = false
            restoreError = ""
            try {
                val saved = loginStore.load()
                canRestore = saved != null
                if (saved != null) connect(saved.connection, saved.refreshToken) { _, _ -> }
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
        AndroidUpdateFrame(this, current?.platform?.env?.dark ?: dark) {
            if (current == null) {
                MaterialTheme(colorScheme = if (dark) darkColorScheme() else lightColorScheme()) {
                    if (restoring) {
                        Surface(Modifier.fillMaxSize()) {
                            Column(
                                Modifier.safeDrawingPadding().padding(24.dp),
                                verticalArrangement = Arrangement.spacedBy(16.dp),
                            ) {
                                Text(if (restoreCancelRequested) "トークン更新の終了を待っています" else "ログインを復元しています")
                                CircularProgressIndicator()
                                TextButton(
                                    enabled = !restoreCancelRequested,
                                    onClick = {
                                        restoreCancelRequested = true
                                        restoreJob?.cancel()
                                    },
                                ) { Text("ログイン画面を開く") }
                            }
                        }
                    } else Login(
                        initialError = restoreError,
                        retryRestore = if (canRestore) ({ restoreAttempt++ }) else null,
                    ) { base, issuer, client, resource, code ->
                        canRestore = false
                        loginStore.clear()
                        connect(LoginConnection(base, issuer, client, resource), null, code)
                    }
                }
            } else {
                val platform = current.platform
                LaunchedEffect(platform.env, dark) {
                    platform.systemDark(dark)
                    current.session.update(platform.env)
                }
                DisposableEffect(current) {
                    val observer = LifecycleEventObserver { _, _ ->
                        platform.visible(lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED))
                    }
                    lifecycle.addObserver(observer)
                    val connectivity = getSystemService(android.net.ConnectivityManager::class.java)
                    fun updateOnline() = runOnUiThread {
                        platform.online(
                            connectivity.activeNetwork?.let {
                                connectivity
                                    .getNetworkCapabilities(it)
                                    ?.hasCapability(
                                        android.net.NetworkCapabilities.NET_CAPABILITY_INTERNET
                                    )
                            } == true
                        )
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
                        lifecycle.removeObserver(observer)
                        connectivity.unregisterNetworkCallback(callback)
                    }
                }
                BackHandler {
                    when {
                        current.session.escape() -> Unit
                        current.session.thread != null -> current.session.thread = null
                        platform.back() -> Unit
                        else -> finish()
                    }
                }
                CompositionLocalProvider(
                    LocalWebPlatform provides platform,
                    LocalApp provides current.session,
                ) {
                    ZooTheme(platform.env.dark, locale = platform.env.locale) {
                        Box(Modifier.fillMaxSize().safeDrawingPadding().imePadding()) {
                            NativeWindow(current.session) { ZooApp(current.session) }
                            if (platform.pendingNavigation != null) {
                                AlertDialog(
                                    onDismissRequest = platform::cancelPendingNavigation,
                                    title = {
                                        Text(platform.translate("channels.notes.unsaved"))
                                    },
                                    confirmButton = {
                                        TextButton(
                                            onClick = {
                                                if (platform.confirmDiscardAndNavigate()) finish()
                                            }
                                        ) {
                                            Text(platform.translate("channels.notes.discardChanges"))
                                        }
                                    },
                                    dismissButton = {
                                        TextButton(onClick = platform::cancelPendingNavigation) {
                                            Text(platform.translate("common.cancel"))
                                        }
                                    },
                                )
                            }
                        }
                    }
                }
            }
        }
    }

    private suspend fun openSession(
        connection: LoginConnection,
        loginStore: AndroidLoginStore,
        dark: Boolean,
        savedRefresh: String?,
        showCode: (String, String) -> Unit,
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
        try {
            if (savedRefresh == null)
                auth.login(connection.issuer, connection.client, connection.resource, showCode)
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
                this, http, auth, connection.base, me.text("id"), logout, ::chooseFiles,
            )
            platform.systemDark(dark)
            val config = Config(
                connection.base, "/api/shapes", me.text("id"), me.flag("is_admin"),
                assetsBaseUrl = connection.base,
            )
            val session = Session(Repository(config, platform, http, lifetime), lifetime, platform.env)
            return RunningSession(session, platform, lifetime, http, auth).also { session.start() }
        } catch (failure: Throwable) {
            auth.close()
            http.close()
            throw failure
        }
    }

    @Composable
    private fun Login(
        initialError: String = "",
        retryRestore: (() -> Unit)? = null,
        connect: suspend (String, String, String, String, (String, String) -> Unit) -> Unit
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
        var code by remember { mutableStateOf("") }
        var link by remember { mutableStateOf("") }
        var error by remember { mutableStateOf(initialError) }
        var job by remember { mutableStateOf<Job?>(null) }
        val scope = rememberCoroutineScope()
        Surface(Modifier.fillMaxSize()) {
            Column(
                Modifier.safeDrawingPadding()
                    .imePadding()
                    .verticalScroll(rememberScrollState())
                    .padding(24.dp),
                verticalArrangement = Arrangement.spacedBy(16.dp),
            ) {
                Text("Zoo", style = MaterialTheme.typography.headlineLarge)
                Text("Android 版 · Compose Multiplatform")
                OutlinedTextField(
                    base,
                    { base = it },
                    label = { Text("Zoo サーバー URL") },
                    enabled = job == null,
                    singleLine = true,
                )
                OutlinedTextField(
                    issuer,
                    { issuer = it },
                    label = { Text("OIDC issuer") },
                    enabled = job == null,
                    singleLine = true,
                )
                OutlinedTextField(
                    client,
                    { client = it },
                    label = { Text("Android 用 public client ID") },
                    enabled = job == null,
                    singleLine = true,
                )
                OutlinedTextField(
                    resource,
                    { resource = it },
                    label = { Text("Resource / audience") },
                    enabled = job == null,
                    singleLine = true,
                )
                Text("サーバー管理者が登録したデバイス認証対応の client ID を指定してください。ブラウザーでコードを承認してログインします。")
                if (error.isNotEmpty()) Text(error, color = MaterialTheme.colorScheme.error)
                if (retryRestore != null && job == null) {
                    TextButton(onClick = retryRestore) { Text("自動ログインを再試行") }
                }
                if (code.isNotEmpty()) {
                    Text("確認コード: $code", style = MaterialTheme.typography.headlineSmall)
                    Text("ブラウザーに表示されたコードが一致することを確認してください。")
                    androidx.compose.material3.Button(
                        onClick = { startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(link))) }
                    ) {
                        Text("認証画面を開く")
                    }
                }
                if (job != null) {
                    CircularProgressIndicator()
                    TextButton(
                        onClick = {
                            job?.cancel()
                            job = null
                            code = ""
                            link = ""
                        }
                    ) {
                        Text("キャンセル")
                    }
                } else {
                    androidx.compose.material3.Button(
                        enabled = client.isNotBlank(),
                        onClick = {
                            job =
                                scope.launch {
                                    error = ""
                                    code = ""
                                    link = ""
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
                                        ) { userCode, uri ->
                                            code = userCode
                                            link = uri
                                        }
                                    } catch (e: CancellationException) {
                                        throw e
                                    } catch (failure: LoginFailure) {
                                        error = failure.message.orEmpty()
                                    } catch (_: Exception) {
                                        error = "ログインできません。接続先・client ID・コードの期限とネットワークを確認してください。"
                                    } finally {
                                        job = null
                                    }
                                }
                        },
                    ) {
                        Text("ログイン")
                    }
                }
            }
        }
    }
}

private class RunningSession(
    val session: Session,
    val platform: AndroidPlatform,
    val scope: CoroutineScope,
    private val http: HttpClient,
    val auth: DeviceAuth,
) {
    fun logout() {
        try { auth.forget() } finally { close() }
    }

    fun close() {
        auth.close()
        session.close()
        scope.cancel()
        http.close()
    }
}
