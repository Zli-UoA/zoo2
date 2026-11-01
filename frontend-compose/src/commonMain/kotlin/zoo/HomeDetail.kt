package zoo

import androidx.compose.foundation.layout.*
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*
import io.ktor.http.encodeURLParameter
import kotlinx.coroutines.*
import kotlinx.serialization.json.JsonNull

/** Profile state belongs to its actor and its requests cannot overwrite a later actor. */
@Composable
fun ActivityPubProfile(app: Session, actorId: String) {
    val scope = rememberCoroutineScope()
    var account by remember(actorId) { mutableStateOf<Row?>(null) }
    var failed by remember(actorId) { mutableStateOf(false) }
    var pending by remember(actorId) { mutableStateOf(false) }
    var request by remember(actorId) { mutableStateOf<Job?>(null) }
    fun load(): Job {
        request?.cancel()
        return scope
            .launch {
                try {
                    account =
                        app.api.request(
                            "/api/activitypub/accounts?actor=${actorId.encodeURLParameter()}"
                        ) as Row
                    failed = false
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    failed = true
                }
            }
            .also { request = it }
    }
    DisposableEffect(app, actorId) {
        load()
        val unsubscribe = app.homeLive.subscribe { load() }
        onDispose {
            unsubscribe()
            request?.cancel()
        }
    }
    Column(
        Modifier.fillMaxWidth().padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        if (failed) {
            Text(
                app.t("home.loadError"),
                Modifier.then(webControlModifier("alert")),
                color = MaterialTheme.colorScheme.error,
            )
            Button(app.t("home.refresh")) { load() }
        }
        account?.let { value ->
            Row(
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                ActivityPubAvatar(
                    value["avatar"] as? Row,
                    value.text("display_name").ifEmpty { value.text("user_name") },
                )
                Column(Modifier.weight(1f)) {
                    ActivityPubText(
                        value.text("display_name").ifEmpty { value.text("user_name") },
                        value.homeRows("emojis"),
                        textStyle = TextStyle(fontSize = 18.sp, fontWeight = FontWeight.SemiBold),
                    )
                    FederatedAccount(
                        value.text("user_name"),
                        value.text("id"),
                        value.flag("remote"),
                    )
                }
            }
            if (value.text("summary").isNotEmpty())
                ActivityPubContent(value.text("summary"), value.homeRows("emojis"))
            FlowRow(
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                val following = value["following"]?.takeUnless { it == JsonNull }
                if (value.flag("remote"))
                    Button(
                        app.t(if (following == null) "home.follow" else "home.unfollow"),
                        enabled = !pending,
                        variant = "outline",
                    ) {
                        pending = true
                        failed = false
                        scope.launch {
                            try {
                                app.api.request(
                                    "/api/activitypub/follows",
                                    if (following == null) "POST" else "DELETE",
                                    obj("actor" to value.text("id")),
                                )
                                load().join()
                            } catch (e: CancellationException) {
                                throw e
                            } catch (_: Exception) {
                                failed = true
                            } finally {
                                pending = false
                            }
                        }
                    }
                if (following != null)
                    Text(
                        app.t(
                            if (value.flag("following")) "home.following" else "home.followPending"
                        ),
                        muted = true,
                    )
                if (!value.flag("remote") && app.config.userId == value.text("id"))
                    NativeLink("/settings?tab=activitypub") {
                        Text(
                            app.t("activitypubSettings.editProfile"),
                            color = LocalLinkColor.current,
                        )
                    }
                if (value.flag("remote") && value.text("url").isNotEmpty())
                    NativeLink(
                        value.text("url"),
                        attributes = mapOf("target" to "_blank", "rel" to "noopener noreferrer"),
                    ) {
                        Text(
                            app.t("activitypubSettings.originalProfile"),
                            color = LocalLinkColor.current,
                        )
                    }
            }
        }
            ?: if (!failed) Text(app.t("home.loading"), Modifier.then(webControlModifier("status")))
            else Unit
    }
    HorizontalDivider()
}
