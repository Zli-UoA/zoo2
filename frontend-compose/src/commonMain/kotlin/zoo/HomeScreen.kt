package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.*
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Tab
import androidx.compose.material3.PrimaryScrollableTabRow
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*
import io.ktor.http.encodeURLParameter
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.collect

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HomeScreen(
    app: Session,
    fixture: Row? = null,
    args: Row = obj(),
    postId: String? = null,
    actorId: String? = null,
) {
    val selectedPost =
        postId
            ?: args.text("postId").takeIf { it.isNotEmpty() }
            ?: homeRouteId(app.env.path, "posts")
    val selectedActor =
        actorId
            ?: args.text("actorId").takeIf { it.isNotEmpty() }
            ?: homeRouteId(app.env.path, "users")
    val scope = rememberCoroutineScope()
    val feed =
        remember(app, selectedPost, selectedActor) {
            HomeTimeline(app, scope, selectedPost, selectedActor)
        }
    val scroll = rememberLazyListState()
    LaunchedEffect(feed, scroll, app) {
        snapshotFlow {
            val layout = scroll.layoutInfo
            val nearEnd = (layout.visibleItemsInfo.lastOrNull()?.index ?: -1) >= layout.totalItemsCount - 3
            nearEnd && layout.totalItemsCount > 0 && feed.hasMore && !feed.loading &&
                !feed.loadingOlder && !feed.refreshing && !feed.error && app.env.visible && app.env.online
        }.collect { ready -> if (ready) feed.older() }
    }
    val composer = rememberHomeComposerState()
    var enabled by remember { mutableStateOf(false) }
    var controlsError by remember { mutableStateOf(false) }
    var follows by remember { mutableStateOf<List<Row>>(emptyList()) }
    var search by remember { mutableStateOf(false) }
    LaunchedEffect(selectedPost, selectedActor) { search = false }
    var reaction by remember { mutableStateOf<Row?>(null) }
    var pending by remember { mutableStateOf(false) }
    var actionError by remember { mutableStateOf("") }
    var delete by remember { mutableStateOf<Row?>(null) }
    var deleteError by remember { mutableStateOf(false) }
    suspend fun controls() = coroutineScope {
        var failed = false
        val status = async {
            try {
                (app.api.request("/api/activitypub/status") as Row).flag("enabled")
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                failed = true
                null
            }
        }
        val relationships = async {
            try {
                app.api.request("/api/activitypub/follows").rows()
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                failed = true
                null
            }
        }
        status.await()?.let { enabled = it }
        relationships.await()?.let { follows = it }
        controlsError = failed
    }
    fun mutate(post: Row, path: String, method: String, payload: Row) {
        if (pending) return
        pending = true
        actionError = ""
        scope.launch {
            try {
                app.api.request(path, method, payload)
                feed.refresh(true)
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                actionError = post.text("id")
            } finally {
                pending = false
            }
        }
    }
    val controlUpdates = remember(app, feed) { CoalescedRefresh(scope, allowed = { app.env.visible && app.env.online }) { controls() } }
    LaunchedEffect(app, feed) {
        feed.restoreSelection()
        feed.refresh()
        controlUpdates.request()
    }
    DisposableEffect(app, feed) {
        val dispose =
            app.homeLive.subscribe {
                feed.refresh(true)
                controlUpdates.request()
            }
        onDispose {
            dispose()
            controlUpdates.close()
            feed.close()
        }
    }
    Box(Modifier.fillMaxSize()) {
        Column(Modifier.fillMaxSize()) {
            ScreenHeader(
                "home",
                app.t(
                    if (selectedPost != null) "activitypubSettings.post"
                    else if (selectedActor != null) "home.userDetails" else "home.title"
                ),
            ) {
                IconButton("user-search", app.t("home.userSearch")) { search = true }
                IconButton("refresh-cw", app.t("home.refresh")) {
                    feed.refresh()
                    controlUpdates.request()
                }
            }
            HorizontalDivider()
            PullToRefreshBox(
                isRefreshing = feed.refreshing && !feed.loading,
                onRefresh = { if (app.env.online && !feed.refreshing) feed.refresh() },
                modifier = Modifier.weight(1f),
            ) {
                LazyColumn(
                    state = scroll,
                    modifier =
                        Modifier.fillMaxSize()
                            .then(
                                webControlModifier(
                                    "feed",
                                    attributes = mapOf("aria-label" to app.t("home.timeline")),
                                )
                            ),
                    contentPadding = PaddingValues(bottom = 96.dp),
                ) {
                    item {
                        HomeWidth {
                            selectedActor?.let { ActivityPubProfile(app, it) }
                            if (selectedPost != null || selectedActor != null)
                                NativeLink("/home", Modifier.padding(12.dp)) {
                                    Text(app.t("home.title"), color = LocalLinkColor.current)
                                }
                            if (selectedPost == null && selectedActor == null) {
                                val timelines = listOf("home", "local", "social")
                                PrimaryScrollableTabRow(
                                    selectedTabIndex = timelines.indexOf(feed.timeline).coerceAtLeast(0),
                                    edgePadding = 16.dp,
                                ) {
                                    timelines.forEach { value ->
                                        Tab(
                                            selected = feed.timeline == value,
                                            onClick = { if (feed.timeline != value) feed.select(value) },
                                            text = {
                                                Text(
                                                    app.t("home.timelineLabels.$value"), maxLines = 1,
                                                    weight = if (feed.timeline == value) FontWeight.SemiBold else FontWeight.Normal,
                                                    color = if (feed.timeline == value) androidx.compose.material3.MaterialTheme.colorScheme.onSurface
                                                        else androidx.compose.material3.MaterialTheme.colorScheme.onSurfaceVariant,
                                                )
                                            },
                                        )
                                    }
                                }
                            }
                            HorizontalDivider()
                        }
                    }
                    if (feed.error)
                        item {
                            HomeWidth {
                                LoadErrorState(app.t("home.loadError"), app.t("home.refresh")) { feed.refresh() }
                            }
                        }
                    if (feed.loading)
                        item {
                            HomeWidth {
                                TimelineLoadingState(app.t("home.loading"))
                            }
                        }
                    else if (feed.posts.isEmpty() && !feed.error)
                        item {
                            HomeWidth {
                                EmptyContentState("home", app.t("home.empty"), app.t("home.emptyHint"))
                            }
                        }
                    items(feed.posts, key = { HomeTimeline.key(it) }) { post ->
                        HomeWidth {
                            val depth = if (selectedPost != null)
                                post.text("tree_depth").toIntOrNull()?.coerceIn(0, 6) ?: 0
                            else 0
                            Box(Modifier.padding(start = (depth * 16).dp)) {
                                HomePostCard(
                                    app,
                                    post,
                                    pending,
                                    actionError == post.text("id"),
                                    reply = { composer.reply(post) },
                                    react = { reaction = post },
                                    toggleReaction = { item ->
                                        mutate(
                                            post,
                                            "/api/activitypub/reactions",
                                            if (item.flag("selected")) "DELETE" else "POST",
                                            obj(
                                                "post_id" to post.text("id"),
                                                "reaction" to item.text("reaction"),
                                                "emoji_id" to (item["emoji"] as? Row)?.get("image_id"),
                                            ),
                                        )
                                    },
                                    repost = {
                                        mutate(
                                            post,
                                            "/api/activitypub/reposts",
                                            if (post.flag("reposted")) "DELETE" else "POST",
                                            obj("post_id" to post.text("id")),
                                        )
                                    },
                                    remove = {
                                        delete = post
                                        deleteError = false
                                    },
                                    edited = { feed.refresh() },
                                    showParent = selectedPost == null,
                                )
                            }
                        }
                    }
                    if (feed.hasMore)
                        item {
                            HomeWidth {
                                Box(
                                    Modifier.fillMaxWidth().padding(16.dp),
                                    contentAlignment = Alignment.Center,
                                ) {
                                    Button(
                                        app.t(if (feed.loadingOlder) "home.loading" else "home.older"),
                                        enabled = !feed.loadingOlder,
                                    ) {
                                        feed.older()
                                    }
                                }
                            }
                        }
                }
            }
        }
        HomeComposer(
            app,
            enabled,
            composer,
            Modifier.align(Alignment.BottomEnd)
                .padding(if (LocalViewportWidth.current >= 768) 32.dp else 20.dp),
        ) { payload ->
            app.api.request("/api/home", "POST", payload)
            feed.refresh()
        }
    }
    reaction?.let { post ->
        HomeReactionPicker(app, post, { reaction = null }) { choice ->
            mutate(
                post,
                "/api/activitypub/reactions",
                "POST",
                obj(
                    "post_id" to post.text("id"),
                    "reaction" to choice.text("reaction"),
                    "emoji_id" to choice["emoji_id"],
                ),
            )
        }
    }
    delete?.let { post ->
        ConfirmationPanel(
            app.t("home.delete"),
            app.t("home.deleteDescription"),
            app.t("home.delete"),
            pending,
            if (deleteError) app.t("home.deleteError") else null,
            { if (!pending) delete = null },
        ) {
            pending = true
            scope.launch {
                try {
                    app.api.request("/api/home/${post.text("id").encodeURLParameter()}", "DELETE")
                    delete = null
                    feed.refresh()
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    deleteError = true
                } finally {
                    pending = false
                }
            }
        }
    }
    if (search)
        HomeUserSearch(app, enabled, follows, controlsError, { search = false }) { controls() }
}

@Composable
private fun HomeWidth(content: @Composable ColumnScope.() -> Unit) {
    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.TopCenter) {
        Column(Modifier.widthIn(max = 672.dp).fillMaxWidth(), content = content)
    }
}

internal fun homeRouteId(path: String, section: String): String? {
    val clean = path.substringBefore('?').substringBefore('#')
    val prefix = "/home/$section/"
    if (!clean.startsWith(prefix)) return null
    return clean.removePrefix(prefix).takeIf { it.isNotEmpty() && '/' !in it }?.decodeURLPart()
}

/** Catalog remounts the real screen to verify local preferences across navigation. */
@Composable
fun HomeStory(app: Session, fixture: Row) {
    val state = fixture.text("state").replace(" ", "")
    var generation by remember(fixture) { mutableStateOf(0) }
    if (state == "PostingWithoutSavedIdentity") {
        val composer = remember { HomeComposerState() }
        Box(Modifier.fillMaxSize()) {
            HomeComposer(app, false, composer, Modifier.align(Alignment.BottomEnd)) {
                app.api.request("/api/home", "POST", it)
            }
        }
    } else Column(Modifier.fillMaxSize()) {
        if (state in listOf("RememberPostSettings", "RememberPostSettingsMobile", "RememberTimeline"))
            Button("画面を再表示") { generation++ }
        key(generation) { HomeScreen(app, fixture, fixture.nested("args")) }
    }
}
