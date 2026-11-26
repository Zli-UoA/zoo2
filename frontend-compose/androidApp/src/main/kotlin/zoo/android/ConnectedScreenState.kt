package zoo.android

import android.util.Base64
import androidx.compose.ui.text.TextRange
import androidx.compose.runtime.*
import io.ktor.http.encodeURLParameter
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
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
    private val instanceColors = mutableStateMapOf<String, Int?>()

    suspend fun loadInstanceColors(hosts: List<String>) {
        for (batch in hosts.filterNot(instanceColors::containsKey).chunked(12)) {
            val query = batch.joinToString(",").encodeURLParameter()
            val response = try {
                app.api.request("/api/activitypub/instance-colors?hosts=$query").jsonObject
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (_: Exception) {
                return
            }
            batch.filter(response::containsKey).forEach { host ->
                instanceColors[host] = response.text(host).removePrefix("#")
                    .takeIf { it.length == 6 }?.toIntOrNull(16)
            }
        }
    }

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
    /** 暗号化した添付として上げる下書き（[channelFiles] のキー）。 */
    private val encryptedFileKeys = mutableSetOf<String>()

    /**
     * チャンネルの添付の下書きを返す。チャンネルの暗号化の状態が、下書きを作ったときと違えば（既存のチャンネルを
     * 暗号化に切り替えた、など）、作り直す（古い上げ方を使い続けないため）。
     */
    private fun attachmentDrafts(key: String, channelId: String): AttachmentDrafts {
        val encrypted = app.isEncryptedChannel(channelId)
        channelFiles[key]?.let { existing ->
            if ((key in encryptedFileKeys) == encrypted) return existing
            existing.close()
        }
        return channelAttachments(channelId).also {
            channelFiles[key] = it
            if (encrypted) encryptedFileKeys += key else encryptedFileKeys -= key
        }
    }

    /**
     * チャンネルの添付の下書き。暗号化チャンネルでは、端末で暗号化してから上げ、本文の `attachments` に入れる値を
     * `encrypted` に持つ（docs/e2ee.md の「暗号化した添付」。平文はサーバーにも S3 にも渡さない）。
     */
    private fun channelAttachments(channelId: String): AttachmentDrafts {
        val encryption = platform.messageEncryption
        val upload: (suspend (Row) -> Row)? = if (!app.isEncryptedChannel(channelId)) null else { file ->
            val cipher = checkNotNull(encryption) { "e2ee.sendUnavailable" }.attachmentCipher
            val bytes = Base64.decode(platform.invoke("fileRead", file.text("id")), Base64.DEFAULT)
            val info = uploadEncryptedAttachment(app.api, cipher, channelId, bytes,
                file.text("name").ifBlank { "attachment" },
                attachmentType(file.text("name"), file.text("type")) ?: "application/octet-stream")
            obj("id" to info.text("attachment_id"), "encrypted" to info)
        }
        return AttachmentDrafts(app, channelId, app.scope, upload).apply { enabled = true }
    }

    /**
     * 暗号化した添付を取得して復号し、端末の共有で開く（ハッシュが合わなければ開かない）。平文の添付ならサーバーが
     * 拒否する種類は、取得も保存もしない（Web の `receivedAttachmentView`）。
     */
    private fun openEncryptedAttachment(raw: String) {
        val encryption = platform.messageEncryption ?: return
        val info = runCatching { kotlinx.serialization.json.Json.parseToJsonElement(raw).jsonObject }.getOrNull() ?: return
        val view = receivedAttachment(info)
        if (view.blocked) { app.toast = app.t("e2ee.attachments.blocked"); return }
        if ((info.text("size").toLongOrNull() ?: Long.MAX_VALUE) > MAX_ATTACHMENT_BYTES) {
            app.toast = app.t("attachments.downloadError")
            return
        }
        app.scope.launch {
            val bytes = try {
                fetchEncryptedAttachment(app.api, encryption.attachmentCipher, info)
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) {
                // 取得や照合の失敗。ハッシュが合わない場合も含むので、中身が変わったおそれを伝える。
                app.toast = app.t("e2ee.attachments.decryptFailed")
                return@launch
            }
            try {
                platform.shareFile(bytes, view.name.ifBlank { "attachment" }, view.mimetype)
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { app.toast = app.t("attachments.downloadError") }
        }
    }

    private var postRequest = 0
    private var currentOverlay by mutableStateOf<String?>(null)
    var overlay: String?
        get() = currentOverlay
        set(value) {
            if (currentOverlay != value) invalidatePostRequest()
            currentOverlay = value
        }
    var inputMenu by mutableStateOf<InputTool?>(null)
    var memberToRemove by mutableStateOf<SelectionItem?>(null)
    private var pins by mutableStateOf<List<String>>(emptyList())
    var dmTargets by mutableStateOf<Set<String>>(emptySet())
        private set
    var encryptedDm by mutableStateOf(false)
    var threadDetails by mutableStateOf<Row>(obj())
        private set
    var editing by mutableStateOf<AndroidUiAction.ActOnEntry?>(null)
        private set
    var mediaPreview by mutableStateOf<TimelineMediaSelection?>(null)
    var confirmingEntry by mutableStateOf<AndroidUiAction.ActOnEntry?>(null)
    private var editingPost = false
    private var returnOverlay: String? = null
    private var postOrigin: String? = null
    var editSelection by mutableStateOf<TextRange?>(null)
        private set
    var replyFocusRequest by mutableIntStateOf(0)
        private set
    var replySelection by mutableStateOf<TextRange?>(null)
        private set
    var editDraft by mutableStateOf("")
        private set
    var openedPostId by mutableStateOf<String?>(null)
        private set
    var people by mutableStateOf<List<Row>>(emptyList())
        private set
    var profile by mutableStateOf<Row>(obj())
        private set
    var profilePosts by mutableStateOf<List<Row>>(emptyList())
        private set
    var followState by mutableStateOf<String?>(null)
        private set
    var federatePost by mutableStateOf(false)
        private set
    var federationAvailable by mutableStateOf(false)
        private set
    var query by mutableStateOf("")
    /** The `/search` field; the controller searches it after typing pauses. */
    var searchText by mutableStateOf(app.globalSearch.query)
    var postSelection by mutableStateOf<TextRange?>(null)
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
    var channelPrivate by mutableStateOf(false)
        private set
    var channelSensitive by mutableStateOf(false)
        private set
    var channelEncrypted by mutableStateOf(false)
        private set
    var postVisibility by mutableStateOf("unlisted")
    private val drafts = mutableStateMapOf<String, InputState>()

    private fun AttachmentDrafts.maxImageSize() {
        acceptedTypes = imageTypes
        maximumBytes = 8L * 1024 * 1024
        enabled = true
    }

    private val workspaceReactionIndex by derivedStateOf {
        workspaceReactionImages(app.config.apiBaseUrl, app.workspaceEmojis)
    }

    fun entry(
        row: Row, users: Map<String, Row>, attachments: List<Row> = emptyList(),
        reactions: List<Row> = emptyList(), previewService: String = "", writable: Boolean = false,
        allowPreview: Boolean = true,
    ): TimelineItem {
        val post = row.nested("post")
        val data = post.takeIf { it.isNotEmpty() } ?: row
        val display = row.nested("display")
        val isPost = post.isNotEmpty() || data.present("visibility")
        val id = data.text("id").ifBlank { data.text("user_id") }
        val name = data.text("display_name").ifBlank {
            users[data.text("user_id")]?.let(::author) ?: data.text("user_name").ifBlank { data.text("user_id") }
        }
        val quoteRow = data.nested("quote")
        val original = quoteRow.nested("post")
        val quoteUri = publicUrl(quoteRow.text("uri"))?.toString()
        val quote = quoteUri?.let { uri ->
            TimelineQuote(
                uri, original.text("id").takeIf { it.isNotBlank() && !original.flag("remote") },
                original.text("display_name").ifBlank { original.text("user_name") },
                original.text("content"), distinctTimelineMedia(original.array("media").rows().mapNotNull(::postMedia)),
                displayEmoji(original, original, isPost = true),
                avatar = timelineAvatarUrl(app.config.apiBaseUrl, original, users[original.text("user_id")]),
            )
        }
        val body = data.text("content").let { content ->
            val rawQuoteUrl = quoteRow.text("uri")
            if (quote == null) content
            else content.trimEnd().removeSuffix("<$rawQuoteUrl>").removeSuffix(rawQuoteUrl).trimEnd()
        }
        val richContent = if (!isPost && !data.present("ciphertext")) messageRichContent(data) else EntryRichContent()
        val metadata = display.ifEmpty { data }
        val emoji = if (allowPreview && !data.present("ciphertext")) displayEmoji(data, metadata, isPost) else emptyMap()
        val reactionEmoji = if (allowPreview && !data.present("ciphertext")) {
            val values = if (isPost) metadata.array("reactions").rows().map { it.text("reaction") }
            else reactions.map { it.text("emoji") }
            values.associateWith { emoji[it]?.url ?: workspaceReactionIndex[it] }
        } else emptyMap()
        val badges = if (isPost) metadata.array("reactions").rows().map {
            val value = it.text("reaction")
            val icon = it.nested("emoji")
            val (label, fallback) = timelineReactionEmoji(app.config.apiBaseUrl, value, reactionEmoji)
            TimelineReaction(value, label, it.text("count"), it.flag("selected"),
                timelineEmojiUrl(app.config.apiBaseUrl, icon) ?: fallback, imageId(icon.text("image_id")),
                canToggle = timelineReactionCanToggle(value, it.flag("selected"), it.text("can_react") != "false", post = true))
        } else reactions.groupBy { it.text("emoji") }.map { (value, rows) ->
            val (label, image) = timelineReactionEmoji(app.config.apiBaseUrl, value, reactionEmoji)
            val account = value.takeIf { it.startsWith(":@") && it.endsWith(':') }
                ?.removePrefix(":@")?.removeSuffix(":")
                ?.let { username -> users.values.firstOrNull { it.text("user_name") == username } }
            TimelineReaction(value, label, rows.size.toString(), rows.any { it.text("user_id") == app.config.userId },
                image ?: account?.let { timelineAvatarUrl(app.config.apiBaseUrl, it) }, canToggle = timelineReactionCanToggle(value, selected = false, canReact = true, post = false))
        }
        val fileRows = (attachments + metadata.array("attachments").rows()).distinctBy { it.text("id") }
        val attachmentMedia = if (allowPreview && !data.present("ciphertext")) fileRows.mapNotNull(::timelineAttachmentMedia)
        else emptyList()
        val previewIds = attachmentMedia.mapNotNull { it.attachmentId }.toSet()
        val files = fileRows.filterNot { it.text("id") in previewIds }.map { it.text("id") to it.text("file_name") }
        val postMedia = data.array("media").rows().mapNotNull(::postMedia)
        // A channel-linked post exposes its attachments through both representations.
        val media = selectTimelineMedia(postMedia, attachmentMedia, isPost && fileRows.isNotEmpty())
        val mediaUrls = timelineImageIdentities(postMedia + quote?.media.orEmpty())
        // このアプリのリンクは自分のデータから埋め込み、外へ送らないので、暗号化チャンネルでも出す。
        // 外部の URL のカードだけを Summaly で取得する。添付として出す画像の URL は、カードにしない。
        val links = if (!richContent.hidesBody) messageLinks(body, app.config.apiBaseUrl, quoteUri, mediaUrls)
            else MessageLinks(emptyList(), null)
        val url = links.external.takeIf { allowPreview && !data.present("ciphertext") }
        val createdAt = data.text("created_at")
        val time = platform.time(createdAt)
        val postHeader = if (isPost) {
            val username = data.text("user_name").ifBlank { users[data.text("user_id")]?.text("user_name").orEmpty() }
            val handle = username.trimStart('@').substringBefore('@').takeIf { it.isNotBlank() }?.let { "@$it" }
            val channelName = data.text("channel_name").trimStart('#').ifBlank { null }
            val remote = data.flag("remote")
            val server = if (remote) {
                publicUrl(data.text("user_id"))?.host ?: publicUrl(data.text("url"))?.host
            } else publicUrl(app.config.apiBaseUrl)?.host
            PostHeaderData(handle, listOf(platform.day(createdAt).substringAfter('/'), time).filter { it.isNotBlank() }.joinToString(" "),
                PostAudience.fromApi(data.text("visibility")), channelName, server,
                remoteServer = remote, serverRgb = if (remote && server != null) instanceColors[server] else null)
        } else null
        return withLinkCard(TimelineItem(id, name, body, time,
            editable = (writable || isPost) && data.text("user_id") == app.config.userId && !data.flag("remote"),
            canReact = writable || isPost,
            canRepost = isPost && data.text("visibility") in setOf("public", "local", "unlisted"),
            reposted = metadata.flag("reposted"), reactions = badges, attachments = files, link = url,
            richContent = richContent, channelLinks = links.channels.map { it.first },
            emoji = emoji, attachedImageUrls = mediaUrls, avatar = timelineAvatarUrl(app.config.apiBaseUrl, data, users[data.text("user_id")]),
            entryKey = HomeTimeline.key(row).ifBlank { id }, media = media, postHeader = postHeader, quote = quote,
            repostedBy = data.text("reposted_by").ifBlank { data.text("reposter_actor") }.ifBlank { null },
        ), previewService)
    }

    fun withLinkCard(entry: TimelineItem, service: String): TimelineItem {
        val url = entry.link ?: return entry
        return entry.copy(linkCard = summalyCard(service, url, entry.attachedImageUrls))
    }

    private fun summalyCard(service: String, url: String, attachedImages: Set<String> = emptySet()): EntryCard? {
        val preview = app.previews.cached(service, url) ?: return null
        val image = publicUrlString(preview.image)?.takeUnless { it in attachedImages }
            ?.let { TimelineMedia(it, "", TimelineMediaKind.Image, url = it) }
        return EntryCard(preview.title, preview.description, url, preview.site, image = image,
            formatted = false, summaly = true)
    }

    private fun displayEmoji(data: Row, metadata: Row, isPost: Boolean): Map<String, TimelineMedia> {
        val inline = if (isPost) data.array("emojis").rows().map { it.text("name") to it }
        else metadata.nested("emojis").ifEmpty { data.nested("display_data").nested("emojis") }
            .mapNotNull { (name, value) -> (value as? JsonObject)?.let { name to it } }
        return buildMap {
            for ((name, emoji) in inline.take(256)) {
                val safe = if (isPost) timelineEmojiUrl(app.config.apiBaseUrl, emoji)
                else timelineImageUrl(app.config.apiBaseUrl, emoji.text("url"))
                if (safe == null || name.isBlank()) continue
                val media = TimelineMedia(safe, name, TimelineMediaKind.Image, url = safe, previewUrl = safe)
                put(":${name.trim(':')}:", media)
                emoji.array("aliases").take(32).mapNotNull { (it as? JsonPrimitive)?.content }
                    .forEach { put(":${it.trim(':')}:", media) }
            }
        }
    }

    private fun postMedia(row: Row): TimelineMedia? = timelinePostMedia(app.config.apiBaseUrl, row)

    private fun runEntryAction(action: AndroidUiAction.ActOnEntry, post: Boolean, encrypted: Boolean) {
        val id = action.item.id.encodeURLParameter()
        val path = if (post) "/api/home/$id" else "/api/messages/$id"
        when (action.operation) {
            EntryOperation.Edit -> if (action.item.editable) {
                returnOverlay = overlay
                editing = action
                editingPost = post
                editDraft = action.item.body
                editSelection = null
                overlay = "edit-entry"
            }
            EntryOperation.Delete -> if (action.item.editable) confirmingEntry = action
            EntryOperation.React -> if (action.item.canReact) {
                if (action.value.isBlank()) {
                    confirmingEntry = action
                } else request {
                    val selectedReaction = action.item.reactions.firstOrNull {
                        it.selected && it.value == action.value &&
                            (action.reactionImageId == null || it.imageId == action.reactionImageId)
                    }
                    val selected = selectedReaction != null
                    if (post) app.api.request("/api/activitypub/reactions", if (selected) "DELETE" else "POST",
                        obj("post_id" to action.item.id, "reaction" to action.value,
                            "emoji_id" to (action.reactionImageId ?: selectedReaction?.imageId)))
                    else if (selected) app.api.request("$path/reactions?emoji=${action.value.encodeURLParameter()}", "DELETE")
                    else app.api.request("$path/reactions", "POST", obj("emoji" to action.value))
                    confirmingEntry = null
                    if (post) refreshPost()
                }
            }
            EntryOperation.Repost -> if (post && action.item.canRepost) request {
                app.api.request("/api/activitypub/reposts", if (action.item.reposted) "DELETE" else "POST",
                    obj("post_id" to action.item.id))
                refreshPost()
            }
            EntryOperation.Attachment -> {
                val info = action.item.encryptedFiles[action.value]
                if (info != null) openEncryptedAttachment(info)
                else action.item.attachments.firstOrNull { it.first == action.value }?.let {
                    app.attachmentLinks.download(obj("id" to it.first, "file_name" to it.second))
                }
            }
            EntryOperation.Link -> if (!encrypted && publicUrl(action.value) != null) platform.action("open:${action.value}")
            EntryOperation.Media -> if (!encrypted) {
                val media = if (action.quotedMedia) action.item.quote?.media.orEmpty() else action.item.media
                mediaPreview = selectMediaPreview(media, action.value)
            }
            EntryOperation.Quote -> action.item.quote?.let { quote ->
                val postId = quote.postId
                if (postId != null) openPost(postId)
                else platform.action("open:${quote.uri}")
            }
        }
    }

    fun confirmDelete(post: Boolean) {
        val action = confirmingEntry?.takeIf { it.operation == EntryOperation.Delete && it.item.editable } ?: return
        request {
            val endpoint = if (post) "home" else "messages"
            app.api.request("/api/$endpoint/${action.item.id.encodeURLParameter()}", "DELETE")
            confirmingEntry = null
            if (post) {
                if (openedPostId == action.item.id) {
                    closePost()
                }
                refreshPost()
            } else if (app.thread == action.item.id) app.thread = null
        }
    }

    private suspend fun refreshPost() {
        home.refresh(retain = true)
        if (overlay == "profile" && profile.text("id").isNotBlank()) {
            val id = profile.text("id")
            val posts = app.api.request("/api/home?timeline=social&author=${id.encodeURLParameter()}&limit=40").rows()
            if (overlay == "profile" && profile.text("id") == id) profilePosts = posts
        }
        val selected = openedPostId ?: return
        val detail = app.api.request("/api/home/${selected.encodeURLParameter()}/thread").rows()
        if (openedPostId == selected) postDetail = detail
    }

    suspend fun searchPeople(term: String) {
        people = if (term.isBlank()) emptyList()
        else app.api.request("/api/activitypub/search?q=${term.encodeURLParameter()}").rows()
    }

    suspend fun loadFederation() {
        federationAvailable = app.api.request("/api/activitypub/status").jsonObject.flag("enabled")
    }

    private suspend fun openProfile(person: Row) {
        profile = person
        val id = person.text("id")
        val remote = person.flag("remote")
        profilePosts = app.api.request("/api/home?timeline=social&author=${id.encodeURLParameter()}&limit=40").rows()
        val account = try {
            app.api.request("/api/activitypub/accounts?actor=${id.encodeURLParameter()}").jsonObject
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (_: Exception) { obj() }
        if (account.isNotEmpty()) profile = kotlinx.serialization.json.JsonObject(person + account)
        followState = if (!remote || id == app.config.userId) null else if (account.isNotEmpty()) {
            when {
                !account.present("following") -> "フォロー"
                account.flag("following") -> "フォローを解除"
                else -> "申請を取り消す"
            }
        } else updateFollowState(id)
        overlay = "profile"
    }

    private suspend fun updateFollowState(actor: String): String {
        val follows = app.api.request("/api/activitypub/follows").rows()
        val followed = follows.firstOrNull { it.text("actor_id") == actor && it.text("direction") == "outgoing" }
        return when {
            followed == null -> "フォロー"
            followed.flag("accepted") -> "フォローを解除"
            else -> "申請を取り消す"
        }
    }

    fun input(key: String, canPost: Boolean, people: List<Row> = emptyList()): InputState {
        val draft = drafts[key] ?: InputState()
        val files = channelFiles[key]
        val position = (draft.selection?.end ?: draft.text.length).coerceIn(0, draft.text.length)
        val mention = Regex("(?:^|\\s)@([^\\s@]*)$").find(draft.text.take(position))?.groupValues?.get(1)
        return draft.copy(
            mentions = if (mention == null) emptyList() else people.asSequence().map { it.text("user_name") }
                .filter { it.isNotBlank() && it.startsWith(mention, ignoreCase = true) }.distinct().take(5).toList(),
            canSend = canPost && !saving && (files == null || files.canSend(draft.text)),
            hasAttachments = files?.items?.isNotEmpty() == true,
            files = files?.items?.map { it.file.text("id") to it.name }.orEmpty(),
        )
    }

    suspend fun refreshPins() {
        pins = app.api.request("/api/channel-pins").jsonObject.array("pins").rows()
            .filterNot { it.flag("archived") }.map { it.text("channel_id") }
    }

    suspend fun loadThread(id: String, operation: String? = null) {
        val path = "/api/threads/${id.encodeURLParameter()}"
        val result = if (operation == null) app.api.request(path)
            else app.api.request("$path/$operation", "POST")
        if (app.thread == id) threadDetails = result.jsonObject.nested("data")
    }

    fun threadSettings(threadId: String, canManage: Boolean, member: Boolean): List<SettingsGroup> {
        val detail = threadDetails.takeIf { it.text("id") == threadId } ?: return emptyList()
        val joined = detail.text("membership") == "joined"
        val archived = detail.present("thread_archive_at")
        return listOf(SettingsGroup("スレッド", buildList {
            if (archived && canManage) add(SettingsEntry.Destination("thread-unarchive", "再開"))
            else if (!archived && member)
                add(SettingsEntry.Destination(if (joined) "thread-leave" else "thread-join", if (joined) "退出" else "参加"))
            add(SettingsEntry.Destination("thread-members", "参加者", detail.text("participant_count").ifBlank { null }))
        }))
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
        val activeById = active.associateBy { it.text("id") }
        val unpinned = active.filterNot { it.text("id") in pins }
        return listOf(
            ChannelSection("ピン留め", pins.mapNotNull(activeById::get).map(::item)),
            ChannelSection("参加中", unpinned.filterNot { it.flag("is_dm") }.map(::item)),
            ChannelSection("ダイレクトメッセージ", unpinned.filter { it.flag("is_dm") }.map(::item)),
        )
    }

    fun searchScreen(
        users: Map<String, Row>, channels: List<Row>, members: List<Row>, previewService: String,
    ): AndroidScreen.Search {
        val search = app.globalSearch
        val channelsById = channels.associateBy { it.text("id") }
        val messagesByChannel = mutableMapOf<String, Map<String, Row>>()
        val year = platform.day(platform.now()).substringBefore('/')
        return AndroidScreen.Search(SearchScreenState(
            searchText, search.query, search.type,
            search.items.filterNot { app.isEncryptedChannel(it.text("channel_id")) }.map { row ->
                val channel = channelsById[row.text("channel_id")]
                val file = isGlobalSearchFile(row, search.type)
                val searchAuthor = row.nested("author")
                val user = users[row.text("user_id")] ?: searchAuthor
                // Only a locally synced full body can establish whether the link is visible.
                val link = if (file || row.text("kind") == "note") null else {
                    messagesByChannel.getOrPut(row.text("channel_id")) {
                        app.repo.cachedMessages(row.text("channel_id")).associateBy { it.text("id") }
                    }[row.text("id")]
                        ?.takeUnless { it.present("ciphertext") || it.text("deleted_at").isNotEmpty() }
                        ?.let { messageLinks(it.text("content"), app.config.apiBaseUrl).external }
                }
                val createdAt = row.text("created_at")
                val day = platform.day(createdAt).let { if (it.substringBefore('/') == year) it.substringAfter('/') else it }
                SearchResultItem(
                    key = globalSearchKey(row, search.type),
                    author = author(user).ifBlank { row.text("user_id") },
                    // Like Web, keep the first hit within the three lines of a narrow screen.
                    body = if (file) row.text("file_name") else focusSnippet(row.text("snippet"), searchTerms(search.query)),
                    time = listOf(day, platform.time(createdAt)).filter { it.isNotBlank() }.joinToString(" "),
                    destination = searchDestination(row, channelsById[row.text("channel_id")], users, members),
                    dm = isDm(row, channelsById[row.text("channel_id")]),
                    inThread = row.text("parent_id").isNotEmpty(),
                    file = file,
                    fileSize = if (file) attachmentSize(row.text("file_size")) else null,
                    emoji = if (file) emptyMap() else displayEmoji(row, row, isPost = false),
                    avatar = timelineAvatarUrl(app.config.apiBaseUrl, user, searchAuthor),
                    media = if (file) searchImage(row, channel) else null,
                    linkCard = link?.let { summalyCard(previewService, it) },
                )
            },
            loading = search.loading, loadingOlder = search.loadingOlder, error = search.error, loaded = search.loaded,
            canLoadOlder = search.nextCursor != null, searchedUntil = search.searchedUntil,
            problem = searchQueryProblem(search.query),
            filters = searchFilters(users, channels),
        ))
    }

    private fun searchImage(row: Row, channel: Row?): TimelineMedia? {
        if (channel == null || app.isEncryptedChannel(row.text("channel_id"))) return null
        val image = timelineAttachmentMedia(row)?.takeIf { it.kind == TimelineMediaKind.Image } ?: return null
        return image.copy(sensitive = image.sensitive || channel.flag("is_nsfw"))
    }

    /** Web's picker choices: the user first, then names in order; channels that search can reach (no DMs or encrypted ones). */
    private fun searchFilters(users: Map<String, Row>, channels: List<Row>): SearchFilterState {
        val filters = app.globalSearch.filters
        val me = app.config.userId
        val people = users.values
            .sortedWith(compareBy<Row>({ it.text("id") != me }, { author(it).lowercase() }))
            .map { user ->
                val name = author(user)
                SearchFilterOption(
                    user.text("id"),
                    if (user.text("id") == me) app.t("globalSearch.filters.me", "name" to name) else name,
                    user.text("user_name").ifBlank { null }?.let { "@$it" },
                )
            }
        val reachable = visibleChannels(channels)
            .filter { !it.flag("is_dm") && !it.flag("encrypted") }
            .map {
                SearchFilterOption(it.text("id"), it.text("channel_name"),
                    mark = if (it.flag("is_private")) SearchFilterMark.PrivateChannel else SearchFilterMark.Channel)
            }
        return SearchFilterState(people, reachable, filters.from, filters.channels, filters.mine, filters.noBots)
    }

    private fun isDm(row: Row, channel: Row?) = row.flag("is_dm") || channel?.flag("is_dm") == true

    /** The channel name (the row shows `#` as an icon); for DMs the other members, as in the channel list. */
    private fun searchDestination(row: Row, channel: Row?, users: Map<String, Row>, members: List<Row>): String {
        if (!isDm(row, channel)) return row.text("channel_name")
        val id = row.text("channel_id")
        val peers = members.filter { it.text("channel_id") == id && it.text("user_id") != app.config.userId }
            .mapNotNull { users[it.text("user_id")]?.let(::author)?.ifBlank { null } }
        return peers.joinToString(", ").ifBlank { channel?.text("channel_name") ?: row.text("channel_name") }
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
            if (member && !channel.present("archive_at")) {
                val pinned = channel.text("id") in pins
                add(SettingsEntry.Destination(if (pinned) "unpin" else "pin", if (pinned) "ピン留めを解除" else "ピン留め"))
            }
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

    fun settings(env: Environment, incomingSettingsAvailable: Boolean): List<SettingsGroup> = listOf(
        SettingsGroup("外観", listOf(SettingsEntry.Destination("theme", "テーマ", env.theme),
            SettingsEntry.Destination("language", "言語", env.locale))),
        SettingsGroup("操作", listOf(SettingsEntry.Toggle("swipe", "スワイプで返信", env.swipeReply),
            SettingsEntry.Toggle("haptics", "触覚フィードバック", env.operationHaptics))),
        SettingsGroup("Zoo", listOf(SettingsEntry.Destination("search-engine", "検索エンジン", app.searchEngine),
            SettingsEntry.Destination("notifications", "通知"),
            SettingsEntry.Destination("app-update", "アプリの更新"),
            SettingsEntry.Destination("encryption", "エンドツーエンド暗号化"),
            SettingsEntry.Destination("logout", "ログアウト"))),
        SettingsGroup("通話", buildList {
            add(SettingsEntry.Destination("sip", app.t("sip.title")))
            add(SettingsEntry.Toggle("dm-calls", "DMの着信を受ける", phone.dmWaiting))
            if (phone.dmWaitingError) add(SettingsEntry.Information("dm-call-error", app.t("androidPhone.callError")))
            if (incomingSettingsAvailable) {
                add(SettingsEntry.Destination("incoming-screen", app.t("androidPhone.incomingScreenSettings")))
            }
        }),
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

    fun dispatch(action: AndroidUiAction, screen: AndroidScreen, channel: Row?, rows: List<Row>, callPeerId: String?, callPeerName: String?, enableNotifications: () -> Unit) {
        val channelId = channel?.text("id").orEmpty()
        val draftKey = channelId + (app.thread?.let { ":$it" } ?: "")
        when (action) {
            is AndroidUiAction.ActOnEntry -> runEntryAction(action,
                screen is AndroidScreen.Home || screen is AndroidScreen.Post || screen is AndroidScreen.Profile,
                channelId.isNotBlank() && app.isEncryptedChannel(channelId))
            is AndroidUiAction.Navigate -> {
                overlay = null
                if (action.page == TopPage.Search) searchText = app.globalSearch.query
                platform.navigate(when (action.page) {
                    TopPage.Home -> "/home"
                    TopPage.Channels -> "/"
                    // The tab reopens the last words and kind; returning keeps the loaded results.
                    TopPage.Search -> globalSearchPath(app.globalSearch.query, app.globalSearch.type, app.globalSearch.filters)
                    TopPage.Notifications -> "/notifications"
                    TopPage.Bubble -> "/bubble"
                    TopPage.Settings -> "/settings"
                })
            }
            is AndroidUiAction.OpenChannel -> { overlay = null; platform.navigate("/channels/${action.id.encodeURLParameter()}") }
            is AndroidUiAction.OpenChannelSettings -> if (action.id == channelId) overlay = "channel-settings"
            is AndroidUiAction.OpenThread -> if (channel != null) app.thread = action.id
            is AndroidUiAction.OpenNotification -> {
                if (action.path.startsWith("post:")) openPost(action.path.removePrefix("post:"))
                else platform.navigate(action.path)
            }
            is AndroidUiAction.RespondFollow -> request {
                app.api.request("/api/activitypub/follows/respond", "POST",
                    obj("actor" to action.actor, "activity_id" to action.activity, "accept" to action.accept))
                app.activityPubNotifications.refresh()
            }
            is AndroidUiAction.SelectInput -> when (screen) {
                is AndroidScreen.EditEntry -> editSelection = action.range
                is AndroidScreen.NewPost -> postSelection = action.range
                is AndroidScreen.Post -> replySelection = action.range
                is AndroidScreen.Conversation, is AndroidScreen.Thread ->
                    drafts[draftKey] = (drafts[draftKey] ?: InputState()).copy(selection = action.range)
                else -> Unit
            }
            is AndroidUiAction.RemoveFile -> {
                val attachments = if (screen is AndroidScreen.NewPost) postFiles else channelFiles[draftKey]
                attachments?.items?.firstOrNull { it.file.text("id") == action.id }?.let { attachments.remove(it) }
            }
            is AndroidUiAction.Mention -> {
                val draft = drafts[draftKey] ?: InputState()
                val end = (draft.selection?.end ?: draft.text.length).coerceIn(0, draft.text.length)
                val target = draft.text.take(end).lastIndexOf('@')
                if (target >= 0) {
                    val mention = "@${action.name} "
                    val text = draft.text.replaceRange(target, end, mention)
                    drafts[draftKey] = draft.copy(text = text, selection = TextRange(target + mention.length))
                }
            }
            AndroidUiAction.RetryNotifications -> app.activityPubNotifications.refresh()
            AndroidUiAction.LoadMoreNotifications -> app.activityPubNotifications.loadMore()
            is AndroidUiAction.ViewedNotification -> inbox.dispatch(action)
            is AndroidUiAction.Toggle -> if (!interaction.dispatch(action) && action.id == "dm-calls") phone.receiveDmCalls(action.value)
            is AndroidUiAction.TextChanged -> when (screen) {
                is AndroidScreen.EditEntry -> editDraft = action.text
                is AndroidScreen.NewPost -> postDraft = action.text
                is AndroidScreen.Post -> replyDraft = action.text
                is AndroidScreen.Bubble -> bubble.edit(action.text)
                else -> drafts[draftKey] = (drafts[draftKey] ?: InputState()).copy(text = action.text)
            }
            is AndroidUiAction.UseInputTool -> inputTool(action.tool, screen, channelId, draftKey, rows)
            is AndroidUiAction.OpenPost -> openPost(action.id)
            is AndroidUiAction.ReplyToPost -> if (screen is AndroidScreen.Post && screen.root.id == action.id) {
                replyFocusRequest++
            } else openPost(action.id, focusReply = true)
            is AndroidUiAction.Search -> query = action.query
            is AndroidUiAction.SearchMessages -> searchText = action.text
            AndroidUiAction.SubmitSearch -> {
                // The IME search key: search the typed words now, or the same words again for new messages.
                val search = app.globalSearch
                if (searchText.trim() == search.query) search.refresh() else search.search(searchText)
            }
            is AndroidUiAction.SelectSearchType -> app.globalSearch.search(searchText, action.type)
            is AndroidUiAction.SelectSearchFilter -> {
                val search = app.globalSearch
                search.search(searchText, filter = when (action.field) {
                    SearchFilterField.From -> search.filters.copy(from = action.ids)
                    SearchFilterField.In -> search.filters.copy(channels = action.ids)
                })
            }
            is AndroidUiAction.ToggleSearchFilter -> {
                val search = app.globalSearch
                search.search(searchText, filter = when (action.flag) {
                    SearchFilterFlag.Mine -> search.filters.copy(mine = !search.filters.mine)
                    SearchFilterFlag.NoBots -> search.filters.copy(noBots = !search.filters.noBots)
                })
            }
            is AndroidUiAction.OpenSearchResult -> {
                val search = app.globalSearch
                search.items.firstOrNull { globalSearchKey(it, search.type) == action.key }
                    // `open` makes each tap a new request: the conversation keeps the last opened target and
                    // would otherwise stay where it was scrolled when the same result is opened again.
                    ?.let { platform.navigate(globalSearchDestination(it, search.type) + "&open=${kotlin.time.Clock.System.now().toEpochMilliseconds()}") }
            }
            is AndroidUiAction.OpenSearchMedia -> {
                val search = app.globalSearch
                val media = search.items.firstOrNull {
                    isGlobalSearchFile(it, search.type) && globalSearchKey(it, search.type) == action.key
                }?.let { row ->
                    val channel = app.repo.channels.rows.value.firstOrNull { it.text("id") == row.text("channel_id") }
                    searchImage(row, channel)
                }
                if (media != null && (!media.sensitive || action.revealedSensitive)) {
                    mediaPreview = TimelineMediaSelection(listOf(media), 0)
                }
            }
            AndroidUiAction.LoadOlderResults -> app.globalSearch.loadOlder()
            AndroidUiAction.RetrySearch -> app.globalSearch.retry()
            is AndroidUiAction.SelectResult -> when (overlay) {
                "people" -> people.firstOrNull { it.text("id") == action.id }?.let { person -> request { openProfile(person) } }
                "add-member" -> if (channel != null) addMember(channel, action.id)
                "browse" -> { overlay = null; platform.navigate("/channels/${action.id.encodeURLParameter()}") }
                "dm" -> {
                    if (action.id in dmTargets) dmTargets -= action.id
                    else if (dmTargets.size < 9) dmTargets += action.id
                }
                else -> Unit
            }
            is AndroidUiAction.OpenSetting -> when {
                action.id == "toggle-federation" -> if (federationAvailable) federatePost = !federatePost else Unit
                action.id == "profile-original" -> {
                    val url = profile.text("url")
                    if (publicUrl(url) != null) platform.action("open:$url")
                }
                action.id == "profile-follow" && followState != null -> request {
                    val actor = profile.text("actor_id").ifBlank { profile.text("id") }
                    app.api.request("/api/activitypub/follows", if (followState == "フォロー") "POST" else "DELETE",
                        obj("actor" to actor))
                    followState = updateFollowState(actor)
                }
                action.id == "submit-dm" && dmTargets.isNotEmpty() && !saving -> request {
                    saving = true
                    try {
                        val id = app.createDm(dmTargets, encryptedDm)
                        overlay = null
                        platform.navigate("/channels/${id.encodeURLParameter()}")
                    } finally { saving = false }
                }
                action.id == "channel-private" -> {
                    channelPrivate = !channelPrivate
                    if (!channelPrivate) channelEncrypted = false
                }
                action.id == "channel-sensitive" -> channelSensitive = !channelSensitive
                action.id == "channel-encrypted" -> if (channelPrivate) channelEncrypted = !channelEncrypted else Unit
                action.id == "encrypted-dm" -> encryptedDm = !encryptedDm
                action.id == "thread-settings" || action.id == "thread-members" -> {
                    query = ""
                    overlay = action.id
                }
                action.id in setOf("thread-join", "thread-leave", "thread-unarchive") ->
                    app.thread?.let { id -> request { loadThread(id, action.id.removePrefix("thread-")) } }
                action.id in setOf("pin", "unpin") && channel != null -> request {
                    if (action.id == "pin") app.api.request("/api/channel-pins", "POST", obj("channel_id" to channelId))
                    else app.api.request("/api/channel-pins/${channelId.encodeURLParameter()}", "DELETE")
                    refreshPins()
                }
                action.id == "os-notifications" -> enableNotifications()
                action.id == "logout" -> request { platform.invoke("logout") }
                action.id.startsWith("theme:") || action.id.startsWith("locale:") -> platform.action(action.id)
                action.id.startsWith("engine:") -> app.searchPreferences.save(action.id.substringAfter(':'))
                action.id == "rename" && channel != null -> {
                    channelName = channel.text("channel_name")
                    channelDescription = channel.text("description")
                    channelPrivate = channel.flag("is_private")
                    channelSensitive = channel.flag("is_nsfw")
                    channelEncrypted = app.isEncryptedChannel(channelId)
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
                    val payload = obj("channel_name" to channelName.trim(), "description" to channelDescription,
                        "is_private" to channelPrivate, "is_nsfw" to channelSensitive)
                    if (overlay == "rename" && channel != null) {
                        app.repo.channels.write(app.api, "/api/channels/${channelId.encodeURLParameter()}", "PATCH", payload)
                    } else app.createChannel(kotlinx.serialization.json.JsonObject(payload + ("encrypted" to JsonPrimitive(channelEncrypted))))
                    overlay = null
                } finally { saving = false }
            }
            AndroidUiAction.Browse -> { query = ""; overlay = "browse" }
            AndroidUiAction.CreateChannel -> {
                channelName = ""
                channelDescription = ""
                channelPrivate = false
                channelSensitive = false
                channelEncrypted = false
                overlay = "create"
            }
            AndroidUiAction.CreateDm -> { query = ""; dmTargets = emptySet(); encryptedDm = false; overlay = "dm" }
            AndroidUiAction.ComposePost -> overlay = "new-post"
            AndroidUiAction.SystemPhotos -> postFiles.pick(photos = true)
            is AndroidUiAction.SelectPhoto -> request { postFiles.add(platform.invoke("pickRecentPhoto", action.id)) }
            is AndroidUiAction.SelectTimeline -> home.select(action.id)
            AndroidUiAction.LoadOlderPosts -> home.older()
            AndroidUiAction.LoadOlderMessages -> if (channelId.isNotBlank()) app.scope.launch {
                app.repo.messages(channelId).loadOlder(app.thread?.let(PageScope::Replies) ?: PageScope.Main)
            }
            is AndroidUiAction.SelectPostVisibility -> postVisibility = action.id
            AndroidUiAction.MinimizeCall, AndroidUiAction.FloatCall -> Unit
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
        val draft = when (screen) {
            is AndroidScreen.EditEntry -> InputState(editDraft, selection = editSelection)
            is AndroidScreen.NewPost -> InputState(postDraft, selection = postSelection)
            is AndroidScreen.Post -> InputState(replyDraft, selection = replySelection)
            else -> drafts[key] ?: InputState()
        }
        val start = (draft.selection?.min ?: draft.text.length).coerceIn(0, draft.text.length)
        val end = (draft.selection?.max ?: draft.text.length).coerceIn(start, draft.text.length)
        val selection = draft.text.substring(start, end)
        val inserted = when {
            selection.isEmpty() -> value
            value == "**太字**" -> "**$selection**"
            value == "*斜体*" -> "*$selection*"
            value == "`コード`" -> "`$selection`"
            value == "\n> 引用" -> "\n" + selection.lineSequence().joinToString("\n") { "> $it" }
            else -> value
        }
        val text = draft.text.replaceRange(start, end, inserted)
        val cursor = TextRange(start + inserted.length)
        when (screen) {
            is AndroidScreen.EditEntry -> { editDraft = text; editSelection = cursor }
            is AndroidScreen.NewPost -> { postDraft = text; postSelection = cursor }
            is AndroidScreen.Post -> { replyDraft = text; replySelection = cursor }
            else -> drafts[key] = draft.copy(text = text, selection = cursor)
        }
        inputMenu = null
    }

    private fun inputTool(tool: InputTool, screen: AndroidScreen, channelId: String, key: String, rows: List<Row>) {
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
                is AndroidScreen.Thread, is AndroidScreen.Conversation -> if (channelId.isNotBlank()) {
                    attachmentDrafts(key, channelId).pick()
                }
                else -> Unit
            }
            InputTool.Send -> if (!saving) request {
                saving = true
                try {
                    when (screen) {
                        is AndroidScreen.Bubble -> bubble.edit("")
                        is AndroidScreen.EditEntry -> {
                            val action = editing ?: return@request
                            val row = rows.firstOrNull { it.text("id") == action.item.id }
                            val post = editingPost
                            if (!post && row == null) return@request
                            if (editDraft.isBlank()) return@request
                            val id = action.item.id.encodeURLParameter()
                            if (!post && app.isEncryptedChannel(channelId)) {
                                val encryption = platform.messageEncryption ?: error("Encryption unavailable")
                                encryption.edit(requireNotNull(row), editDraft)
                            } else {
                                if (!post) {
                                    app.requirePlaintextChannel(channelId)
                                }
                                val endpoint = if (post) "home" else "messages"
                                app.api.request("/api/$endpoint/$id", "PATCH", obj("content" to editDraft))
                            }
                            finishEditing()
                            if (post) refreshPost()
                        }
                        is AndroidScreen.NewPost -> {
                            val sent = postFiles.items.toList()
                            val text = postDraft
                            val ids = sent.mapNotNull { it.uploaded?.text("id") }
                            if (!postFiles.canSend(postDraft)) return@request
                            app.api.request("/api/home", "POST", obj("content" to text,
                                "apub" to (federatePost && federationAvailable && postVisibility != "local"), "visibility" to postVisibility, "image_ids" to JsonArray(ids.map(::JsonPrimitive))))
                            val sentIds = sent.map { it.file.text("id") }.toSet()
                            postFiles.items.removeAll { it.file.text("id") in sentIds }
                            postFiles.release(sent)
                            if (postDraft == text) {
                                postDraft = ""
                                postSelection = null
                            }
                            if (overlay == "new-post" && postFiles.items.isEmpty() && postDraft.isBlank()) overlay = null
                            home.refresh(retain = true)
                        }
                        is AndroidScreen.Post -> {
                            val text = replyDraft
                            if (text.isBlank()) return@request
                            app.api.request("/api/home", "POST", obj("content" to text,
                                "visibility" to replyVisibility(screen.root.id), "apub" to false, "in_reply_to" to screen.root.id))
                            if (overlay == "post" && openedPostId == screen.root.id && replyDraft == text) {
                                replyDraft = ""
                                replySelection = null
                            }
                            val detail = app.api.request("/api/home/${screen.root.id.encodeURLParameter()}/thread").rows()
                            if (overlay == "post" && openedPostId == screen.root.id) postDetail = detail
                        }
                        is AndroidScreen.Conversation, is AndroidScreen.Thread -> {
                            val files = channelFiles[key]
                            val draft = drafts[key] ?: InputState()
                            val sent = files?.items?.toList().orEmpty()
                            val uploaded = sent.mapNotNull { it.uploaded }
                            // 暗号化チャンネルの添付は、暗号化して上げたときの本文の `attachments` の値を渡す。
                            val encryptedFiles = uploaded.mapNotNull { it["encrypted"] as? JsonObject }
                            val ids = uploaded.filter { it["encrypted"] == null }.map { it.text("id") }
                            if (uploaded.size == sent.size && app.send(channelId, draft.text, app.thread, attachmentIds = ids,
                                    encryptedAttachments = encryptedFiles)) {
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

    /** 本文のリンクと埋め込みカードから、アプリ内でチャンネル・メッセージへ移動する。 */
    fun openChannelLink(target: ChannelLinkTarget, parentId: String?) {
        overlay = null
        app.openChannelLink(target, parentId)
    }

    private fun replyVisibility(id: String): String {
        val row = postDetail.map { it.nested("post").ifEmpty { it } }.firstOrNull { it.text("id") == id }
        return when (row?.text("visibility")) {
            "followers" -> "followers"
            "local" -> "local"
            else -> "unlisted"
        }
    }

    fun invalidatePostRequest() { postRequest++ }

    private fun openPost(id: String, focusReply: Boolean = false) {
        val opening = ++postRequest
        val source = overlay
        val path = app.env.path
        request {
            val detail = app.api.request("/api/home/${id.encodeURLParameter()}/thread").rows()
            if (detail.isNotEmpty() && opening == postRequest && source == overlay && path == app.env.path) {
                if (source != "post") postOrigin = source
                if (openedPostId != id) {
                    replyDraft = ""
                    replySelection = null
                }
                postDetail = detail
                openedPostId = id
                replyFocusRequest = if (focusReply) replyFocusRequest + 1 else 0
                overlay = "post"
            }
        }
    }

    fun closePost() {
        invalidatePostRequest()
        openedPostId = null
        postDetail = emptyList()
        replyDraft = ""
        replySelection = null
        replyFocusRequest = 0
        overlay = postOrigin
        postOrigin = null
    }

    fun finishEditing() {
        editing = null
        editDraft = ""
        editSelection = null
        overlay = returnOverlay
        returnOverlay = null
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
