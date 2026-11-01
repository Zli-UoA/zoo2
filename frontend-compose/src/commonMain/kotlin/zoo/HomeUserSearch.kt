package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.*
import io.ktor.http.encodeURLParameter
import kotlinx.coroutines.*

@Composable
fun HomeUserSearch(
    app: Session,
    enabled: Boolean,
    follows: List<Row>,
    controlsError: Boolean,
    close: () -> Unit,
    reload: suspend () -> Unit,
) {
    val scope = rememberCoroutineScope()
    var query by remember { mutableStateOf("") }
    var results by remember { mutableStateOf<List<Row>?>(null) }
    var loading by remember { mutableStateOf(false) }
    var failed by remember { mutableStateOf(false) }
    var pending by remember { mutableStateOf(false) }
    Panel(app.t("home.userSearch"), close, 512.dp) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Field(
                query,
                app.t("home.userSearch"),
                Modifier.weight(1f),
                placeholder = "alice / @user@example.social",
            ) {
                query = it
            }
            Button(app.t("home.search"), enabled = !loading && query.trim().isNotEmpty()) {
                loading = true
                failed = false
                scope.launch {
                    try {
                        results =
                            app.api
                                .request(
                                    "/api/activitypub/search?q=${query.trim().encodeURLParameter()}"
                                )
                                .rows()
                    } catch (e: CancellationException) {
                        throw e
                    } catch (_: Exception) {
                        failed = true
                    } finally {
                        loading = false
                    }
                }
            }
        }
        if (loading) Text(app.t("home.loading"))
        if (failed || controlsError)
            Text(app.t("home.controlsError"), Modifier.then(webControlModifier("alert")))
        Column(
            Modifier.fillMaxWidth().heightIn(max = 480.dp).verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            if (results?.isEmpty() == true) Text(app.t("home.noUsers"))
            results?.forEach { user ->
                Column(
                    Modifier.fillMaxWidth()
                        .border(
                            1.dp,
                            MaterialTheme.colorScheme.outlineVariant,
                            androidx.compose.foundation.shape.RoundedCornerShape(8.dp),
                        )
                        .padding(16.dp)
                        .then(
                            webControlModifier(
                                "article",
                                attributes = mapOf("aria-label" to app.t("home.userDetails")),
                            )
                        ),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Avatar(user, size = 32.dp)
                        Column {
                            ActivityPubText(
                                user.text("display_name").ifEmpty { user.text("user_name") },
                                user.homeRows("emojis"),
                            )
                            FederatedAccount(
                                user.text("user_name"),
                                user.text("id"),
                                user.flag("remote"),
                            )
                        }
                    }
                    if (user.present("created_at"))
                        Text(
                            app.t(
                                "home.joinedAt",
                                "date" to app.platform.fullTimestamp(user.text("created_at")),
                            ),
                            size = 12.sp,
                        )
                    if (publicUrl(user.text("url")) != null)
                        NativeLink(
                            user.text("url"),
                            attributes = mapOf("target" to "_blank", "rel" to "noopener noreferrer"),
                        ) {
                            Text(user.text("url"), color = LocalLinkColor.current)
                        }
                    if (user.flag("remote")) {
                        val follow =
                            follows.find {
                                it.text("actor_id") == user.text("id") &&
                                    it.text("direction") == "outgoing"
                            }
                        Button(
                            app.t(if (follow != null) "home.unfollow" else "home.follow"),
                            enabled = enabled && !pending,
                        ) {
                            pending = true
                            failed = false
                            scope.launch {
                                try {
                                    app.api.request(
                                        "/api/activitypub/follows",
                                        if (follow != null) "DELETE" else "POST",
                                        obj("actor" to user.text("id")),
                                    )
                                    reload()
                                } catch (e: CancellationException) {
                                    throw e
                                } catch (_: Exception) {
                                    failed = true
                                } finally {
                                    pending = false
                                }
                            }
                        }
                        if (follow != null)
                            Text(
                                app.t(
                                    if (follow.flag("accepted")) "home.following"
                                    else "home.followPending"
                                ),
                                size = 12.sp,
                            )
                    }
                }
            }
        }
    }
}
