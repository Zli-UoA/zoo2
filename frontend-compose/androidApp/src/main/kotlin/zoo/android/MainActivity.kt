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
import zoo.phone.LocalIncomingScreenSettings
import zoo.phone.LocalPhonePictureInPicture
import zoo.android.phone.PhonePictureInPictureActivity
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
