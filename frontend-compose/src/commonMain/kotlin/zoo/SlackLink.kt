package zoo

import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.*
import kotlinx.serialization.json.*

/** Slack からの戻り (`?code=...&state=...` または `?slack=error&reason=...`)。React の `readSlackLinkResult`。 */
sealed interface SlackLinkResult {
    data class Authorized(val code: String, val state: String) : SlackLinkResult

    data class Failed(val reason: String) : SlackLinkResult
}

fun readSlackLinkResult(query: Map<String, String?>): SlackLinkResult? {
    val code = query["code"]
    val state = query["state"]
    if (code != null && state != null) return SlackLinkResult.Authorized(code, state)
    if (query["slack"] == "error") return SlackLinkResult.Failed(query["reason"].orEmpty())
    return null
}

/** `complete` の失敗理由。サーバーの HTTP ステータスに対応する (React の `slackLinkErrorReason`) */
fun slackLinkErrorReason(status: Int?): String =
    when (status) {
        400 -> "invalid_state"
        403 -> "team_mismatch"
        409 -> "already_linked"
        else -> "failed"
    }

private val slackLinkErrors = setOf("denied", "invalid_state", "team_mismatch", "already_linked")

/** 各ユーザーによる Slack アカウントの連携 (#291)。React の `SlackLinkPanel` に対応する。 */
class SlackLinkController(
    parent: CoroutineScope,
    private val request: suspend (path: String, method: String, body: JsonElement?) -> JsonElement,
) {
    private val lifetime = SupervisorJob(parent.coroutineContext[Job])
    private val scope = CoroutineScope(parent.coroutineContext + lifetime)
    // 連携の完了・解除で状態を書き換えるたびに進める。それより前に始めた読み込みの結果は捨てる。
    private var revision = 0

    var state by mutableStateOf<Row?>(null)
        private set

    /** 表示する失敗の翻訳キー */
    var error by mutableStateOf<String?>(null)
        private set

    var pending by mutableStateOf(false)
        private set

    /** 連携に成功したら true (画面が通知を出す) */
    var linked by mutableStateOf(0)
        private set

    var unlinked by mutableStateOf(0)
        private set

    fun load() {
        if (!lifetime.isActive) return
        val started = revision
        scope.launch {
            try {
                val value = request("/api/slack/link", "GET", null).jsonObject
                if (revision == started) state = value
            } catch (cause: CancellationException) {
                throw cause
            } catch (_: Exception) {
                if (revision == started) error = "slackLink.loadError"
            }
        }
    }

    /** 認可 URL を受け取り、`navigate` で Slack へ移動する */
    fun link(navigate: suspend (String) -> Unit) {
        if (pending || !lifetime.isActive) return
        pending = true
        error = null
        scope.launch {
            try {
                navigate(
                    request("/api/slack/oauth/start", "POST", null).jsonObject.text("authorize_url")
                )
            } catch (cause: CancellationException) {
                throw cause
            } catch (_: Exception) {
                error = "slackLink.errors.failed"
                pending = false
            }
        }
    }

    /** Slack から戻ったときの結果を処理する。完了に失敗したら状態を読み直して、やり直せるようにする。 */
    fun handle(result: SlackLinkResult) {
        if (!lifetime.isActive) return
        when (result) {
            is SlackLinkResult.Failed ->
                error =
                    "slackLink.errors." +
                        (result.reason.takeIf { it in slackLinkErrors } ?: "failed")
            is SlackLinkResult.Authorized -> {
                pending = true
                revision++
                scope.launch {
                    try {
                        state =
                            request(
                                    "/api/slack/oauth/complete",
                                    "POST",
                                    obj("code" to result.code, "state" to result.state),
                                )
                                .jsonObject
                        linked++
                    } catch (cause: CancellationException) {
                        throw cause
                    } catch (cause: Exception) {
                        error =
                            "slackLink.errors." +
                                slackLinkErrorReason((cause as? ApiException)?.status)
                        // 初期の読み込みは捨てているので、連携をやり直せるよう状態を読み直す
                        load()
                    } finally {
                        pending = false
                    }
                }
            }
        }
    }

    fun unlink(done: () -> Unit) {
        if (pending || !lifetime.isActive) return
        pending = true
        error = null
        scope.launch {
            try {
                val next = request("/api/slack/link", "DELETE", null).jsonObject
                revision++
                state = next
                unlinked++
                done()
            } catch (cause: CancellationException) {
                throw cause
            } catch (_: Exception) {
                error = "slackLink.unlinkFailed"
            } finally {
                pending = false
            }
        }
    }

    fun close() {
        revision++
        lifetime.cancel()
    }
}

@Composable
fun SlackLinkPanel(
    app: Session,
    navigate: suspend (String) -> Unit = { app.platform.invoke("redirect", it) },
) {
    val scope = rememberCoroutineScope()
    val state =
        remember(app) {
            SlackLinkController(scope) { path, method, body -> app.api.request(path, method, body) }
        }
    LaunchedEffect(state) { state.load() }
    DisposableEffect(state) { onDispose { state.close() } }
    // Slack から戻ってきたら連携を完了し、URL から code / state を消す
    val query = app.query
    val result = readSlackLinkResult(query.names().associateWith { query[it] })
    var handled by remember { mutableStateOf<SlackLinkResult?>(null) }
    LaunchedEffect(result) {
        if (result == null || result == handled) return@LaunchedEffect
        handled = result
        app.platform.replace("/settings?tab=slack")
        state.handle(result)
    }
    LaunchedEffect(state.linked) {
        if (state.linked > 0) app.showToast(app.t("slackLink.linked"), ToastKind.Success)
    }
    LaunchedEffect(state.unlinked) {
        if (state.unlinked > 0) app.showToast(app.t("slackLink.unlinked"), ToastKind.Success)
    }
    var confirmUnlink by remember { mutableStateOf(false) }
    val current = state.state
    val link = current?.get("link") as? JsonObject

    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(16.dp)) {
        SettingsSection(app.t("slackLink.title"), app.t("slackLink.description")) {
            if (current == null && state.error == null)
                Text(
                    app.t("slackLink.loading"),
                    Modifier.then(webControlModifier("status")).semantics {
                        liveRegion = LiveRegionMode.Polite
                    },
                    muted = true,
                )
            if (current != null && !current.flag("configured"))
                Text(app.t("slackLink.notConfigured"), small = true, muted = true)
            if (current != null && current.flag("configured") && link != null)
                CssFlowRow(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        app.t(
                            "slackLink.linkedAs",
                            "name" to
                                link.text("slack_user_name").ifEmpty { link.text("slack_user_id") },
                            "team" to link.text("team_name").ifEmpty { link.text("team_id") },
                        ),
                        small = true,
                    )
                    Button(
                        app.t("slackLink.unlink"),
                        variant = "outline",
                        icon = "unlink",
                        enabled = !state.pending,
                    ) {
                        confirmUnlink = true
                    }
                }
            if (current != null && current.flag("configured") && link == null)
                Button(
                    app.t("slackLink.link"),
                    primary = true,
                    icon = "link",
                    enabled = !state.pending,
                ) {
                    state.link(navigate)
                }
            state.error?.let {
                Text(
                    app.t(it),
                    Modifier.then(webControlModifier("alert")),
                    small = true,
                    color = MaterialTheme.colorScheme.error,
                )
            }
        }
        if (app.config.isAdmin) SlackManifestSection(app)
    }
    if (confirmUnlink)
        ConfirmationPanel(
            app.t("slackLink.unlinkTitle"),
            app.t("slackLink.unlinkDescription"),
            app.t("slackLink.unlink"),
            state.pending,
            state.error?.takeIf { it == "slackLink.unlinkFailed" }?.let { app.t(it) },
            { confirmUnlink = false },
            { state.unlink { confirmUnlink = false } },
        )
}

/** 管理者向け: ワークスペースで作る Slack アプリの App Manifest */
@Composable
private fun SlackManifestSection(app: Session) {
    var manifest by remember { mutableStateOf("") }
    var configured by remember { mutableStateOf(true) }
    var failed by remember { mutableStateOf(false) }
    LaunchedEffect(app) {
        try {
            val value = app.api.request("/api/admin/slack/manifest").jsonObject
            manifest = prettyJson.encodeToString(JsonElement.serializer(), value.nested("manifest"))
            configured = value.flag("configured")
        } catch (cause: CancellationException) {
            throw cause
        } catch (_: Exception) {
            failed = true
        }
    }
    SettingsSection(app.t("slackLink.manifestTitle"), app.t("slackLink.manifestDescription")) {
        if (!configured) Text(app.t("slackLink.manifestNotConfigured"), small = true, muted = true)
        if (manifest.isNotEmpty()) {
            Field(
                manifest,
                app.t("slackLink.manifestTitle"),
                Modifier.fillMaxWidth(),
                singleLine = false,
                readOnly = true,
                showLabel = false,
                minimumHeight = 192.dp,
                maximumHeight = 192.dp,
            ) {}
            Button(app.t("slackLink.copyManifest"), variant = "outline", icon = "copy") {
                app.scope.launch {
                    try {
                        app.platform.invoke("copy", manifest)
                        app.showToast(app.t("slackLink.manifestCopied"), ToastKind.Success)
                    } catch (cause: CancellationException) {
                        throw cause
                    } catch (_: Exception) {
                        app.showToast(app.t("slackLink.manifestCopyFailed"), ToastKind.Error)
                    }
                }
            }
        }
        if (failed)
            Text(
                app.t("slackLink.manifestError"),
                Modifier.then(webControlModifier("alert")),
                small = true,
                color = MaterialTheme.colorScheme.error,
            )
    }
}

private val prettyJson = Json { prettyPrint = true }
