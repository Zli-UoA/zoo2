package zoo

import androidx.compose.foundation.*
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.*
import io.ktor.http.*
import kotlinx.coroutines.CancellationException
import kotlinx.serialization.json.*

internal fun isPublicProfilePath(path: String): Boolean =
    Regex("^/public/users/[^/]+/?$").matches(path.substringBefore('?').substringBefore('#'))

/** Match the actor origin used by WebFinger, omitting default HTTP(S) ports. */
internal fun publicAccountAddress(userName: String, actor: String): String {
    val url = Url(actor)
    val host = if (url.port == url.protocol.defaultPort) {
        url.hostWithPort.removeSuffix(":${url.port}")
    } else url.hostWithPort
    return "@$userName@$host"
}

/** The profile identity and bio also used in the authenticated Home profile. */
@Composable
fun ActivityPubProfileDetails(value: Row, headingLevel: Int = 2) {
    Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
        ActivityPubAvatar(value["avatar"] as? Row, value.text("display_name").ifEmpty { value.text("user_name") })
        Column(Modifier.weight(1f)) {
            Box(Modifier.then(webControlModifier("heading", attributes = mapOf("aria-level" to "$headingLevel")))) {
                ActivityPubText(value.text("display_name").ifEmpty { value.text("user_name") }, value.homeRows("emojis"),
                    textStyle = TextStyle(fontSize = 18.sp, fontWeight = FontWeight.SemiBold))
            }
            FederatedAccount(value.text("user_name"), value.text("id"), value.flag("remote"))
        }
    }
    if (value.text("summary").isNotEmpty()) ActivityPubContent(value.text("summary"), value.homeRows("emojis"))
}

@Composable
fun PublicProfileScreen(app: Session, userId: String = app.env.path.substringBefore('?').substringBefore('#').removePrefix("/public/users/").trimEnd('/').decodeURLPart()) {
    val before = app.query["before"]
    var data by remember(userId, before) { mutableStateOf<Row?>(null) }
    var error by remember(userId, before) { mutableStateOf<Int?>(null) }
    var attempt by remember { mutableStateOf(0) }
    LaunchedEffect(userId, before, attempt) {
        data = null
        error = null
        try {
            val query = before?.let { "?before=${it.encodeURLParameter()}" }.orEmpty()
            data = app.api.request("/api/activitypub/users/${userId.encodeURLParameter()}/profile$query").jsonObject
        } catch (e: CancellationException) { throw e }
        catch (e: Exception) { error = (e as? ApiException)?.status ?: 500 }
    }
    PublicProfileView(app, userId, data, error, before) { attempt++ }
}

@Composable
fun PublicProfileView(app: Session, userId: String, data: Row?, error: Int?, before: String?, retry: () -> Unit) {
    val colors = MaterialTheme.colorScheme
    val shape = RoundedCornerShape(16.dp)
    fun Modifier.card() = fillMaxWidth().background(colors.surface, shape).border(1.dp, colors.outlineVariant, shape).padding(20.dp)
    Column(Modifier.fillMaxSize().background(colors.surfaceVariant).verticalScroll(rememberScrollState()).then(webControlModifier("main"))) {
        Column(Modifier.widthIn(max = 672.dp).fillMaxWidth().align(Alignment.CenterHorizontally).padding(horizontal = 16.dp, vertical = 24.dp), verticalArrangement = Arrangement.spacedBy(24.dp)) {
            NativeLink("/", attributes = mapOf("aria-label" to "Zoo")) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    ZooMark(40.dp)
                    Row {
                        Text("Zoo", size = 40.sp, lineHeight = 40.sp, weight = FontWeight.ExtraBold, letterSpacing = (-2.4).sp)
                        Text(".", size = 40.sp, lineHeight = 40.sp, weight = FontWeight.ExtraBold, color = colors.primary)
                    }
                }
            }
            if (data != null) {
                val account = data["account"]!!.jsonObject
                val actor = account.text("actor_id")
                val address = publicAccountAddress(account.text("user_name"), actor)
                Column(Modifier.card(), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    ActivityPubProfileDetails(JsonObject(account + obj("id" to actor, "remote" to true)), 1)
                    androidx.compose.material3.HorizontalDivider()
                    Text(app.t("publicProfile.followTitle"), weight = FontWeight.SemiBold)
                    Text(app.t("publicProfile.followDescription"))
                    Field(address, app.t("publicProfile.accountAddress"), readOnly = true) {}
                    Text(app.t("publicProfile.approvalRequired"), muted = true)
                    NativeLink("/home/users/${userId.encodeURLParameter()}") {
                        Button(app.t("publicProfile.openInZoo"), primary = true) { app.navigate("/home/users/${userId.encodeURLParameter()}") }
                    }
                }
                Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                    Text(app.t("publicProfile.posts"), weight = FontWeight.SemiBold)
                    val posts = data.homeRows("posts")
                    posts.forEach { post ->
                        Column(Modifier.card().then(webControlModifier("article"))) {
                            Text(app.platform.fullTimestamp(post.text("created_at")), size = 14.sp, muted = true)
                            ActivityPubContent(post.text("content"), modifier = Modifier.padding(top = 8.dp), textStyle = TextStyle(fontSize = 14.sp, lineHeight = 20.sp))
                        }
                    }
                    if (posts.isEmpty()) Text(app.t("publicProfile.empty"), muted = true)
                    val path = "/public/users/${userId.encodeURLParameter()}"
                    FlowRow(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        if (before != null) NativeLink(path) { Button(app.t("publicProfile.latest"), variant = "outline") { app.navigate(path) } }
                        val cursor = data.text("next_cursor")
                        if (cursor.isNotEmpty()) {
                            val older = "$path?before=${cursor.encodeURLParameter()}"
                            NativeLink(older) { Button(app.t("publicProfile.older"), variant = "outline") { app.navigate(older) } }
                        }
                    }
                }
            } else if (error != null) {
                Column(Modifier.card().then(webControlModifier("alert")), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Text(app.t(if (error == 404) "publicProfile.notFound" else "home.loadError"))
                    if (error != 404) Button(app.t("home.refresh"), variant = "outline", click = retry)
                }
            } else Text(app.t("home.loading"), Modifier.then(webControlModifier("status")))
        }
    }
}
