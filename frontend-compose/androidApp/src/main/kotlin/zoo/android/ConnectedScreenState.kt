package zoo.android

import android.util.Base64
import androidx.compose.runtime.*
import io.ktor.http.encodeURLParameter
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import zoo.*
import zoo.android.ui.*
import zoo.phone.PhoneController

internal class ConnectedScreenState(
    private val app: Session,
    private val platform: AndroidPlatform,
    private val phone: PhoneController,
) {
    val home = HomeTimeline(app, app.scope)
    val inbox = NotificationInboxBinding(zoo.NotificationInbox(app))
    val interaction = InteractionSettingsBinding(app)
    val photoBytes = mutableStateMapOf<String, ByteArray>()
    val postFiles = AttachmentDrafts(app, "", app.scope) { file ->
        attachmentPayload(app.api.request("/api/home/images", "POST", obj(
            "name" to file.text("name"), "content_type" to attachmentType(file.text("name"), file.text("type")),
            "data" to platform.invoke("fileRead", file.text("id")),
        )))
    }.apply {
        maxImageSize()
        onDiscard = { row ->
            app.scope.launch {
                try { app.api.request("/api/home/images/${row.text("id").encodeURLParameter()}", "DELETE") }
                catch (cancelled: CancellationException) { throw cancelled }
                catch (_: Exception) { }
            }
        }
    }
    private val channelFiles = mutableMapOf<String, AttachmentDrafts>()
    var overlay by mutableStateOf<String?>(null)
    var inputMenu by mutableStateOf<InputTool?>(null)
    var memberToRemove by mutableStateOf<SelectionItem?>(null)
    var query by mutableStateOf("")
    var postDraft by mutableStateOf("")
    var replyDraft by mutableStateOf("")
    val bubble = BubbleController(app)
    var photosOpen by mutableStateOf(false)
    var recent by mutableStateOf<List<RecentPhoto>>(emptyList())
    var postDetail by mutableStateOf<List<Row>>(emptyList())
    var saving by mutableStateOf(false)
        private set
    var channelName by mutableStateOf("")
    var channelDescription by mutableStateOf("")
    var postVisibility by mutableStateOf("unlisted")
    private val drafts = mutableStateMapOf<String, InputState>()

    private fun AttachmentDrafts.maxImageSize() {
        acceptedTypes = imageTypes
        maximumBytes = 8L * 1024 * 1024
        enabled = true
    }

    fun entry(row: Row, users: Map<String, Row>): TimelineItem {
        val name = row.text("display_name").ifBlank {
            users[row.text("user_id")]?.let(::author) ?: row.text("user_name").ifBlank { row.text("user_id") }
        }
        return TimelineItem(row.text("id"), name, row.text("content"), platform.time(row.text("created_at")))
    }

    fun input(key: String, canPost: Boolean): InputState {
        val draft = drafts[key] ?: InputState()
        val files = channelFiles[key]
        return draft.copy(
            canSend = canPost && !saving && (files == null || files.canSend(draft.text)),
            hasAttachments = files?.items?.isNotEmpty() == true,
        )
    }

    fun channelSections(channels: List<Row>, users: Map<String, Row>, members: List<Row>): List<ChannelSection> {
        val joined = members.filter { it.text("user_id") == app.config.userId }.map { it.text("channel_id") }.toSet()
        val active = visibleChannels(channels).filter { it.text("id") in joined && !it.present("archive_at") }
        fun item(row: Row): ChannelItem {
            val id = row.text("id")
            val peers = members.filter { it.text("channel_id") == id && it.text("user_id") != app.config.userId }
            val userName = peers.singleOrNull()?.let { users[it.text("user_id")] }?.let(::author)
                ?.takeIf { row.flag("is_dm") && it.isNotBlank() }
            return ChannelItem(id, userName ?: row.text("channel_name"), id in app.unread, userName != null)
        }
        return listOf(
            ChannelSection("参加中", active.filterNot { it.flag("is_dm") }.map(::item)),
            ChannelSection("ダイレクトメッセージ", active.filter { it.flag("is_dm") }.map(::item)),
        )
    }

    fun canEditMembers(channel: Row): Boolean = !channel.flag("is_dm") &&
        !channel.present("archive_at") && canManage(channel, app.config.userId, app.config.isAdmin)

    fun addMember(channel: Row, userId: String) {
        if (!canEditMembers(channel)) return
        request {
            app.api.request("/api/channels/${channel.text("id").encodeURLParameter()}/users", "POST", obj("user_id" to userId))
            query = ""
            overlay = "members"
        }
    }

    fun removeMember(channel: Row, userId: String) {
        if (!canEditMembers(channel) || userId == app.config.userId) return
        request {
            app.api.request("/api/channels/${channel.text("id").encodeURLParameter()}/users/${userId.encodeURLParameter()}", "DELETE")
            memberToRemove = null
        }
    }

    fun channelSettings(channel: Row, member: Boolean, encrypted: Boolean): List<SettingsGroup> {
        val entries = buildList {
            if (channel.text("description").isNotBlank()) add(SettingsEntry.Information("description", channel.text("description")))
            if (encrypted) add(SettingsEntry.Information("encrypted", "暗号化チャンネル"))
            add(SettingsEntry.Destination("members", "メンバー"))
            if (canEditMembers(channel)) add(SettingsEntry.Destination("add-member", "メンバーを追加"))
            if (canManage(channel, app.config.userId, app.config.isAdmin) && !channel.flag("is_dm")) {
                val archived = channel.present("archive_at")
                add(SettingsEntry.Destination(if (archived) "unarchive" else "archive",
                    if (archived) "アーカイブを解除" else "アーカイブ"))
            }
            if (canManage(channel, app.config.userId, app.config.isAdmin) &&
                !channel.present("archive_at") && !channel.flag("is_dm")) {
                add(SettingsEntry.Destination("rename", "名前と説明を編集"))
            }
            if (!channel.flag("is_dm") && !channel.present("archive_at")) {
                if (member) add(SettingsEntry.Destination("leave", "退出"))
                else if (!channel.flag("is_private")) add(SettingsEntry.Destination("join", "参加"))
            }
        }
        return listOf(SettingsGroup("チャンネル", entries))
    }

    fun settings(env: Environment, dmWaiting: Boolean): List<SettingsGroup> = listOf(
        SettingsGroup("外観", listOf(SettingsEntry.Destination("theme", "テーマ", env.theme),
            SettingsEntry.Destination("language", "言語", env.locale))),
        SettingsGroup("操作", listOf(SettingsEntry.Toggle("swipe", "スワイプで返信", env.swipeReply),
            SettingsEntry.Toggle("haptics", "触覚フィードバック", env.operationHaptics))),
        SettingsGroup("Zoo", listOf(SettingsEntry.Destination("search-engine", "検索エンジン", app.searchEngine),
            SettingsEntry.Destination("notifications", "通知"),
            SettingsEntry.Toggle("dm-calls", "DMの着信を受ける", dmWaiting),
            SettingsEntry.Destination("logout", "ログアウト"))),
    )

    fun settingsDetail(env: Environment): AndroidScreen {
        val (title, entries) = when (overlay) {
            "theme" -> "テーマ" to listOf("system" to "端末に従う", "light" to "ライト", "dark" to "ダーク")
                .map { SettingsEntry.Destination("theme:${it.first}", it.second, if (env.theme == it.first) "選択中" else null) }
            "language" -> "言語" to listOf("ja" to "日本語", "en" to "English", "ko-KR" to "한국어", "ko-KP" to "조선말",
                "zh-Hans" to "简体中文", "zh-Hant" to "繁體中文", "yue" to "粵語").map {
                SettingsEntry.Destination("locale:${it.first}", it.second, if (env.locale == it.first) "選択中" else null)
            }
            "search-engine" -> "検索エンジン" to searchEngines.map {
                SettingsEntry.Destination("engine:${it.key}", it.value, if (app.searchEngine == it.key) "選択中" else null)
            }
            else -> "通知" to listOf(SettingsEntry.Destination("os-notifications", "Androidの通知設定を開く"))
        }
        return AndroidScreen.SettingsDetail(title, listOf(SettingsGroup(title, entries)))
    }

    fun dispatch(action: AndroidUiAction, screen: AndroidScreen, channel: Row?, callPeerId: String?, callPeerName: String?, enableNotifications: () -> Unit) {
        val channelId = channel?.text("id").orEmpty()
        val draftKey = channelId + (app.thread?.let { ":$it" } ?: "")
        when (action) {
            is AndroidUiAction.Navigate -> {
                overlay = null
                platform.navigate(when (action.page) {
                    TopPage.Home -> "/home"
                    TopPage.Channels -> "/"
                    TopPage.Notifications -> "/notifications"
                    TopPage.Bubble -> "/bubble"
                    TopPage.Settings -> "/settings"
                })
            }
            is AndroidUiAction.OpenChannel -> { overlay = null; platform.navigate("/channels/${action.id.encodeURLParameter()}") }
            is AndroidUiAction.OpenChannelSettings -> if (action.id == channelId) overlay = "channel-settings"
            is AndroidUiAction.OpenThread -> if (channel != null) app.thread = action.id
            is AndroidUiAction.OpenNotification -> platform.navigate(action.path)
            AndroidUiAction.LoadMoreNotifications -> app.activityPubNotifications.loadMore()
            is AndroidUiAction.ViewedNotification -> inbox.dispatch(action)
            is AndroidUiAction.Toggle -> if (!interaction.dispatch(action) && action.id == "dm-calls") phone.receiveDmCalls(action.value)
            is AndroidUiAction.TextChanged -> when (screen) {
                is AndroidScreen.NewPost -> postDraft = action.text
                is AndroidScreen.Post -> replyDraft = action.text
                is AndroidScreen.Bubble -> bubble.edit(action.text)
                else -> drafts[draftKey] = (drafts[draftKey] ?: InputState()).copy(text = action.text)
            }
            is AndroidUiAction.UseInputTool -> inputTool(action.tool, screen, channelId, draftKey)
            is AndroidUiAction.OpenPost -> request {
                postDetail = app.api.request("/api/home/${action.id.encodeURLParameter()}/thread").rows()
                if (postDetail.isNotEmpty()) overlay = "post"
            }
            is AndroidUiAction.Search -> query = action.query
            is AndroidUiAction.SelectResult -> when (overlay) {
                "add-member" -> if (channel != null) addMember(channel, action.id)
                "browse" -> { overlay = null; platform.navigate("/channels/${action.id.encodeURLParameter()}") }
                "dm" -> request {
                    val data = app.api.request("/api/dms", "POST", obj("target_user_ids" to JsonArray(listOf(JsonPrimitive(action.id))))).jsonObject
                    val id = data.nested("data").text("id").ifBlank { data.text("id") }
                    if (id.isNotBlank()) { overlay = null; platform.navigate("/channels/${id.encodeURLParameter()}") }
                }
                else -> Unit
            }
            is AndroidUiAction.OpenSetting -> when {
                action.id == "os-notifications" -> enableNotifications()
                action.id == "logout" -> request { platform.invoke("logout") }
                action.id.startsWith("theme:") || action.id.startsWith("locale:") -> platform.action(action.id)
                action.id.startsWith("engine:") -> app.searchPreferences.save(action.id.substringAfter(':'))
                action.id == "rename" && channel != null -> {
                    channelName = channel.text("channel_name")
                    channelDescription = channel.text("description")
                    overlay = "rename"
                }
                action.id in setOf("join", "leave") && channel != null -> request {
                    app.api.request("/api/channels/${channelId.encodeURLParameter()}/${action.id}", "POST")
                    overlay = null
                }
                else -> { query = ""; overlay = action.id }
            }
            is AndroidUiAction.ChannelNameChanged -> channelName = action.name
            is AndroidUiAction.ChannelDescriptionChanged -> channelDescription = action.description
            AndroidUiAction.SubmitChannel -> if (!saving) request {
                saving = true
                try {
                    val payload = obj("channel_name" to channelName.trim(), "description" to channelDescription, "is_private" to false)
                    if (overlay == "rename" && channel != null) {
                        app.repo.channels.write(app.api, "/api/channels/${channelId.encodeURLParameter()}", "PATCH",
                            obj("channel_name" to channelName.trim(), "description" to channelDescription))
                    } else app.createChannel(payload)
                    overlay = null
                } finally { saving = false }
            }
            AndroidUiAction.Browse -> { query = ""; overlay = "browse" }
            AndroidUiAction.CreateChannel -> { channelName = ""; channelDescription = ""; overlay = "create" }
            AndroidUiAction.CreateDm -> { query = ""; overlay = "dm" }
            AndroidUiAction.ComposePost -> overlay = "new-post"
            AndroidUiAction.SystemPhotos -> postFiles.pick(photos = true)
            is AndroidUiAction.SelectPhoto -> request { postFiles.add(platform.invoke("pickRecentPhoto", action.id)) }
            is AndroidUiAction.SelectTimeline -> home.select(action.id)
            is AndroidUiAction.SelectPostVisibility -> postVisibility = action.id
            AndroidUiAction.Answer -> phone.answer()
            AndroidUiAction.HangUp -> phone.hangup()
            AndroidUiAction.MuteCall -> phone.mute()
            AndroidUiAction.SpeakerCall -> phone.speaker()
            AndroidUiAction.StartCall -> if (channel != null && channel.flag("is_dm")) {
                phone.callDm(channelId, callPeerName.orEmpty().ifBlank { channel.text("channel_name") }, callPeerId)
            }
            AndroidUiAction.SignIn, AndroidUiAction.Back -> Unit
        }
    }

    fun archiveChannel(channel: Row, operation: String) {
        if (channel.flag("is_dm") || !canManage(channel, app.config.userId, app.config.isAdmin)) return
        request {
            app.api.request("/api/channels/${channel.text("id").encodeURLParameter()}/$operation", "POST")
            overlay = "channel-settings"
        }
    }

    fun insertText(screen: AndroidScreen, channelId: String, value: String) {
        val key = channelId + (app.thread?.let { ":$it" } ?: "")
        when (screen) {
            is AndroidScreen.NewPost -> postDraft += value
            is AndroidScreen.Post -> replyDraft += value
            else -> {
                val draft = drafts[key] ?: InputState()
                drafts[key] = draft.copy(text = draft.text + value)
            }
        }
        inputMenu = null
    }

    private fun inputTool(tool: InputTool, screen: AndroidScreen, channelId: String, key: String) {
        when (tool) {
            InputTool.Expand -> (drafts[key] ?: InputState()).let { drafts[key] = it.copy(expanded = !it.expanded) }
            InputTool.Attach -> when (screen) {
                is AndroidScreen.NewPost -> {
                    photosOpen = !photosOpen
                    if (photosOpen) request {
                        var data = kotlinx.serialization.json.Json.parseToJsonElement(platform.invoke("recentPhotos", "")).jsonObject
                        if (data.flag("supported") && !data.flag("allowed")) {
                            data = kotlinx.serialization.json.Json.parseToJsonElement(platform.invoke("requestRecentPhotos", "")).jsonObject
                        }
                        photoBytes.clear()
                        recent = data.array("photos").rows().mapNotNull {
                            val id = it.text("id")
                            if (id.isBlank()) return@mapNotNull null
                            photoBytes[id] = Base64.decode(it.text("thumbnail"), Base64.DEFAULT)
                            RecentPhoto(id, it.text("name"))
                        }
                    }
                }
                is AndroidScreen.Thread, is AndroidScreen.Conversation -> if (channelId.isNotBlank() && !app.isEncryptedChannel(channelId)) {
                    channelFiles.getOrPut(key) { AttachmentDrafts(app, channelId, app.scope).apply { enabled = true } }.pick()
                }
                else -> Unit
            }
            InputTool.Send -> if (!saving) request {
                saving = true
                try {
                    when (screen) {
                        is AndroidScreen.NewPost -> {
                            val ids = postFiles.items.mapNotNull { it.uploaded?.text("id") }
                            if (!postFiles.canSend(postDraft)) return@request
                            app.api.request("/api/home", "POST", obj("content" to postDraft,
                                "apub" to false, "visibility" to postVisibility, "image_ids" to JsonArray(ids.map(::JsonPrimitive))))
                            val sent = postFiles.items.toList()
                            postFiles.items.clear()
                            postFiles.release(sent)
                            postDraft = ""
                            overlay = null
                            home.refresh(retain = true)
                        }
                        is AndroidScreen.Post -> {
                            if (replyDraft.isBlank()) return@request
                            app.api.request("/api/home", "POST", obj("content" to replyDraft,
                                "visibility" to "unlisted", "apub" to false, "in_reply_to" to screen.root.id))
                            replyDraft = ""
                            postDetail = app.api.request("/api/home/${screen.root.id.encodeURLParameter()}/thread").rows()
                        }
                        is AndroidScreen.Conversation, is AndroidScreen.Thread -> {
                            val files = channelFiles[key]
                            val draft = drafts[key] ?: InputState()
                            val sent = files?.items?.toList().orEmpty()
                            val ids = sent.mapNotNull { it.uploaded?.text("id") }
                            if (ids.size == sent.size && app.send(channelId, draft.text, app.thread, attachmentIds = ids)) {
                                drafts[key] = InputState()
                                files?.items?.clear()
                                files?.release(sent)
                            }
                        }
                        else -> Unit
                    }
                } finally { saving = false }
            }
            InputTool.Format, InputTool.Emoji -> inputMenu = tool
        }
    }

    private fun request(block: suspend () -> Unit) {
        if (!platform.env.online) return
        app.scope.launch {
            try { block() }
            catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { app.toast = app.t("common.errors.network") }
        }
    }

    fun close() {
        home.close()
        postFiles.close()
        channelFiles.values.forEach { it.close() }
    }
}
