package zoo

import androidx.compose.foundation.layout.*
import androidx.compose.runtime.*
import kotlinx.coroutines.launch
import kotlinx.serialization.json.*

@Composable
fun AppStory(app: Session, options: Row) {
    val fixture = options.nested("fixture")
    fun callback(name: String, args: JsonArray = JsonArray(emptyList())) {
        if (fixture.array("callbacks").any { it.jsonPrimitive.content == name })
            app.scope.launch {
                app.platform.invoke("storyCallback", obj("name" to name, "args" to args).toString())
            }
    }
    when (fixture.text("storyTitle")) {
        "Features/Apps/Blocks",
        "Flows/Apps/SubmitAndRetry" ->
            Blocks(
                app,
                fixture["blocks"],
                fixture.text("appId"),
                fixture.text("surfaceId").ifEmpty { null },
                fixture.text("messageId").ifEmpty { null },
                fixture.text("channelId").ifEmpty { null },
                fixture.flag("submit"),
                done = {
                    if (fixture.array("callbacks").any { it.jsonPrimitive.content == "onDone" })
                        app.platform.invoke(
                            "storyCallback",
                            obj("name" to "onDone", "args" to JsonArray(emptyList())).toString(),
                        )
                },
            )
        "Features/Apps/Connections" ->
            AppConnections(
                app,
                fixture.text("appId"),
                fixture.text("channelId").ifEmpty { null },
                fixture.flag("connections"),
                fixture.flag("assistant"),
            )
        "Features/Apps/Catalog" ->
            Apps(
                app,
                initial = fixture.text("initialApp").ifEmpty { null },
                embedded = true,
                initialSearch = fixture.text("initialSearch"),
                initialManagement = fixture.flag("initialManagement"),
                channelId = fixture.text("channelId").ifEmpty { null },
                close = { callback("onClose") },
            )
        "Pages/AppConsent" -> ZooApp(app)
        "Features/Channels/Webhooks" ->
            Webhooks(
                app,
                fixture.nested("channel"),
                close = { callback("onOpenChange", JsonArray(listOf(JsonPrimitive(false)))) },
            )
        else -> error("Unregistered App story: ${fixture.text("storyTitle")}")
    }
}
