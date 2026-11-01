package zoo

import androidx.compose.foundation.*
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

@Composable
fun HomePostCard(
    app: Session,
    post: Row,
    pending: Boolean,
    error: Boolean,
    reply: () -> Unit,
    react: () -> Unit,
    toggleReaction: (Row) -> Unit,
    repost: () -> Unit,
    remove: () -> Unit,
    edited: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    var editing by remember(post.text("id")) { mutableStateOf(false) }
    var draft by remember(post.text("id")) { mutableStateOf("") }
    var saving by remember { mutableStateOf(false) }
    var editError by remember { mutableStateOf(false) }
    val own = post.text("user_id") == app.config.userId && !post.flag("remote")
    Column(
        Modifier.fillMaxWidth().then(webControlModifier("article")).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (post.present("reposted_by"))
            Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                Icon("repeat-2", modifier = Modifier.size(12.dp))
                Text(
                    app.t("home.repostedBy", "name" to post.text("reposted_by")),
                    size = 12.sp,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            val profile = "/home/users/${post.text("user_id").encodeURLParameter()}"
            NativeLink(profile) {
                ActivityPubAvatar(
                    if (post.flag("remote")) post["avatar"] as? Row else null,
                    post.text("display_name").ifEmpty { post.text("user_name") },
                )
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                FlowRow(
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalArrangement = Arrangement.spacedBy(2.dp),
                ) {
                    NativeLink(profile) {
                        ActivityPubText(
                            post.text("display_name").ifEmpty { post.text("user_name") },
                            post.homeRows("actor_emojis"),
                            textStyle =
                                TextStyle(fontSize = 14.sp, fontWeight = FontWeight.SemiBold),
                        )
                    }
                    NativeLink(profile) {
                        FederatedAccount(
                            post.text("user_name"),
                            post.text("user_id"),
                            post.flag("remote"),
                        )
                    }
                    NativeLink("/home/posts/${post.text("id").encodeURLParameter()}") {
                        Text(
                            app.platform.fullTimestamp(post.text("created_at")),
                            Modifier.then(
                                webControlModifier(
                                    "time",
                                    attributes = mapOf("datetime" to post.text("created_at")),
                                )
                            ),
                            size = 12.sp,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    if (!post.flag("remote") && post.text("channel_id").isNotEmpty())
                        NativeLink(
                            "/channels/${post.text("channel_id")}?message=${post.text("id")}"
                        ) {
                            Text(
                                "#${post.text("channel_name")}",
                                size = 12.sp,
                                color = LocalLinkColor.current,
                            )
                        }
                }
                if (editing) {
                    Field(
                        draft,
                        app.t("home.edit"),
                        Modifier.fillMaxWidth(),
                        singleLine = false,
                        enabled = !saving,
                    ) {
                        draft = it
                    }
                    if (editError)
                        Text(
                            app.t("home.controlsError"),
                            Modifier.then(webControlModifier("alert")),
                        )
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(
                            app.t("home.save"),
                            enabled =
                                !saving &&
                                    draft.trim().isNotEmpty() &&
                                    homePostLength(draft) <= 40000,
                            primary = true,
                        ) {
                            saving = true
                            scope.launch {
                                try {
                                    app.api.request(
                                        "/api/home/${post.text("id").encodeURLParameter()}",
                                        "PATCH",
                                        obj("content" to draft),
                                    )
                                    editing = false
                                    edited()
                                } catch (e: CancellationException) {
                                    throw e
                                } catch (_: Exception) {
                                    editError = true
                                } finally {
                                    saving = false
                                }
                            }
                        }
                        Button(app.t("common.cancel"), enabled = !saving) { editing = false }
                    }
                } else
                    ActivityPubContent(
                        post.text("content").ifEmpty { app.t("home.attachmentsOnly") },
                        post.homeRows("emojis"),
                        textStyle = TextStyle(fontSize = 14.sp, lineHeight = 20.sp),
                    )
                if (post.homeRows("reactions").isNotEmpty())
                    FlowRow(
                        horizontalArrangement = Arrangement.spacedBy(4.dp),
                        verticalArrangement = Arrangement.spacedBy(4.dp),
                    ) {
                        post.homeRows("reactions").forEach { item ->
                            val label = "${item.text("reaction")} ${item.text("count")}"
                            Row(
                                Modifier.then(
                                        webControlModifier(
                                            "button",
                                            enabled = !pending,
                                            selected = item.flag("selected"),
                                            attributes = mapOf("aria-label" to label),
                                        )
                                    )
                                    .background(
                                        if (item.flag("selected"))
                                            MaterialTheme.colorScheme.secondaryContainer
                                        else MaterialTheme.colorScheme.surface,
                                        androidx.compose.foundation.shape.RoundedCornerShape(6.dp),
                                    )
                                    .border(
                                        1.dp,
                                        MaterialTheme.colorScheme.outlineVariant,
                                        androidx.compose.foundation.shape.RoundedCornerShape(6.dp),
                                    )
                                    .clickable(enabled = !pending) { toggleReaction(item) }
                                    .padding(horizontal = 8.dp, vertical = 4.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(4.dp),
                            ) {
                                (item["emoji"] as? Row)?.let { ActivityPubEmoji(it, 18.dp) }
                                    ?: Text(item.text("reaction"))
                                Text(item.text("count"), size = 12.sp)
                            }
                        }
                    }
                Row(
                    Modifier.fillMaxWidth()
                        .then(
                            webControlModifier(
                                "group",
                                attributes = mapOf("aria-label" to app.t("home.actions")),
                            )
                        ),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Button(
                        post.text("replies").ifEmpty { "0" },
                        icon = "reply",
                        accessibleLabel =
                            "${app.t("home.reply")} ${post.text("replies").ifEmpty{"0"}}",
                        variant = "ghost",
                        size = "sm",
                        click = reply,
                    )
                    Button(
                        post.text("reposts").ifEmpty { "0" },
                        icon = "repeat-2",
                        accessibleLabel =
                            "${app.t(if(post.flag("reposted"))"home.undoRepost" else "home.repost")} ${post.text("reposts").ifEmpty{"0"}}",
                        selected = post.flag("reposted"),
                        enabled = !pending && post.text("visibility") != "followers",
                        variant = "ghost",
                        size = "sm",
                        click = repost,
                    )
                    Button(
                        post.text("likes").ifEmpty { "0" },
                        icon = "smile-plus",
                        accessibleLabel =
                            "${app.t("home.react")} ${post.text("likes").ifEmpty{"0"}}",
                        enabled = !pending,
                        variant = "ghost",
                        size = "sm",
                        click = react,
                    )
                    Menu(
                        app.t("home.postActions"),
                        buildList {
                            if (own) {
                                add(
                                    MenuAction("pencil", app.t("home.edit")) {
                                        draft = post.text("content")
                                        editing = true
                                        editError = false
                                    }
                                )
                                add(MenuAction("trash-2", app.t("home.delete"), !pending, remove))
                            }
                            add(
                                MenuAction("copy", app.t("home.copyContent")) {
                                    app.run { app.platform.invoke("copy", post.text("content")) }
                                }
                            )
                        },
                    )
                }
                if (error)
                    Text(app.t("home.controlsError"), Modifier.then(webControlModifier("alert")))
                if (post.flag("remote") && publicUrl(post.text("url")) != null)
                    NativeLink(
                        post.text("url"),
                        attributes = mapOf("target" to "_blank", "rel" to "noopener noreferrer"),
                    ) {
                        Text(app.t("home.original"), size = 12.sp, color = LocalLinkColor.current)
                    }
            }
        }
    }
    HorizontalDivider()
}
