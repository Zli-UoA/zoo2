package zoo

import androidx.compose.foundation.layout.*
import androidx.compose.material3.HorizontalDivider
import androidx.compose.runtime.*
import androidx.compose.ui.*
import androidx.compose.ui.semantics.*
import androidx.compose.ui.unit.*
import kotlinx.serialization.json.*

@Composable
fun NotificationStory(app: Session, options: Row) {
    val fixture = options.nested("fixture")
    val users = fixture.array("users").map { it.jsonObject }
    val notifications = fixture.array("notifications").map { it.jsonObject }
    when (fixture.text("view")) {
        "row" ->
            NotificationCard(
                app,
                notifications,
                users,
                true,
                fixture.flag("readEnabled"),
                {},
                channelOverride = fixture["channel"] as? JsonObject,
                channelProvided = true,
                membershipsOverride = fixture.array("members").map { it.jsonObject },
                onRead = { ids ->
                    app.run {
                        app.platform.invoke(
                            "storyCallback",
                            obj(
                                    "name" to "onRead",
                                    "args" to
                                        JsonArray(listOf(JsonArray(ids.map { JsonPrimitive(it) }))),
                                )
                                .toString(),
                        )
                    }
                },
            )
        "screen" ->
            if (fixture.flag("comparison"))
                BoxWithConstraints(Modifier.fillMaxSize()) {
                    val wide = maxWidth >= 768.dp
                    @Composable
                    fun comparison() {
                        Column(
                            Modifier.fillMaxWidth().then(webControlModifier("region")).semantics {
                                contentDescription = fixture.text("comparisonLabel")
                            }
                        ) {
                            Header(
                                app,
                                fixture.array("channels").first().jsonObject,
                                users,
                                fixture.array("channelUsers").map { it.jsonObject },
                                !wide,
                                wide,
                            )
                            HorizontalDivider()
                            Box(Modifier.padding(12.dp)) {
                                RichText(fixture.text("comparisonText"), users)
                            }
                        }
                    }
                    if (wide)
                        Row(Modifier.fillMaxSize()) {
                            Box(Modifier.weight(1f)) { comparison() }
                            Box(Modifier.weight(1f)) { Notifications(app, notifications, users) }
                        }
                    else
                        Column(Modifier.fillMaxSize()) {
                            Box(Modifier.weight(1f)) { comparison() }
                            Box(Modifier.weight(1f)) { Notifications(app, notifications, users) }
                        }
                }
            else Notifications(app, notifications, users)
        "button" ->
            NotificationsButton(
                app,
                notifications.count {
                    it.text("user_id") == app.config.userId && !it.flag("is_read")
                },
            )
        "sync" -> SyncStatus(app, fixture.nested("config"))
        "offline" ->
            OfflineBanner(app, fixture.nested("offline")) {
                if (!fixture.nested("offline").flag("isOnline")) app.platform.action("reload")
                else
                    app.run {
                        app.platform.invoke(
                            "storyCallback",
                            obj("name" to "retryPersistence", "args" to JsonArray(emptyList()))
                                .toString(),
                        )
                    }
            }
        "skeleton" -> ShellSkeleton()
        else -> error("Unknown notification story view")
    }
}
