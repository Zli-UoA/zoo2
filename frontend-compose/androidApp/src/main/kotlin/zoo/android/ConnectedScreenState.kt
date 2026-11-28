package zoo.android

import android.util.Base64
import androidx.compose.ui.text.TextRange
import androidx.compose.runtime.*
import io.ktor.http.encodeURLParameter
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
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
    val inbox = NotificationInboxBinding(app)
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

    suspend fun receiveShare(share: IncomingShare, channelId: String?) {
        val files = if (share.images.isEmpty()) null else platform.registerFiles(share.images)
        fun append(original: String) = listOf(original, share.text).filter { it.isNotBlank() }.joinToString("\n")
        if (channelId == null) {
            postDraft = append(postDraft)
            postSelection = TextRange(postDraft.length)
            overlay = "new-post"
            if (files != null) postFiles.add(files)
        } else {
            val old = drafts[channelId] ?: InputState()
            val text = append(old.text)
            drafts[channelId] = old.copy(text = text, selection = TextRange(text.length))
            app.thread = null
            overlay = null
            platform.navigate("/channels/${channelId.encodeURLParameter()}")
            if (files != null) attachmentDrafts(channelId, channelId).add(files)
        }
    }

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
    private fun openEncryptedAttachment(raw: String, share: Boolean) {
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
                if (share) platform.shareFile(bytes, view.name.ifBlank { "attachment" }, view.mimetype)
                else platform.saveFile(bytes, view.name.ifBlank { "attachment" }, view.mimetype)
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) { app.toast = app.t("attachments.downloadError") }
        }
    }

    var feedScreenRevision by mutableIntStateOf(0)
        private set
    private var currentOverlay by mutableStateOf<String?>(null)
    var overlay: String?
        get() = currentOverlay
        set(value) {
            if (currentOverlay != value) {
                feedScreenRevision++
            }
            currentOverlay = value
        }
    var inputMenu by mutableStateOf<InputTool?>(null)
    var memberToRemove by mutableStateOf<SelectionItem?>(null)
    private var pins by mutableStateOf<List<String>>(emptyList())
    private data class DmSelection(val members: Set<String>, val encrypted: Boolean)
    private val pendingDms = mutableStateMapOf<DmSelection, Row>()

    fun pendingDm(id: String?): Row? = pendingDms.values.firstOrNull { it.text("id") == id }

    fun clearSyncedDms(channels: List<Row>, memberships: List<Row>) {
        if (pendingDms.isEmpty()) return
        val joined = memberships.asSequence().filter { it.text("user_id") == app.config.userId }
            .map { it.text("channel_id") }.toSet()
        val synced = channels.asSequence().map { it.text("id") }.filter { it in joined }.toSet()
        pendingDms.entries.removeAll { it.value.text("id") in synced }
    }
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
    private sealed interface FeedOrigin {
        data class Surface(val overlay: String?) : FeedOrigin
        data class Post(val id: String, val draft: String, val selection: TextRange?,
            val focusRequest: Int, val lastSend: String?) : FeedOrigin
        data class Profile(val account: Row) : FeedOrigin
    }
    private val feedBackStack = ArrayDeque<FeedOrigin>()
    var postRevision by mutableIntStateOf(0)
        private set
    private var postDetailRevision by mutableIntStateOf(-1)
    val postPending: Boolean get() = postDetailRevision != postRevision
    var postReloadRequest by mutableIntStateOf(0)
        private set
    var postLoadFailed by mutableStateOf(false)
        private set
    var editSelection by mutableStateOf<TextRange?>(null)
        private set
    var replyLastSend by mutableStateOf<String?>(null)
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
    private var profilePostsRevision by mutableIntStateOf(0)
    val profilePostsPending: Boolean get() = profilePostsRevision != postRevision
    var profilePostsLoadFailed by mutableStateOf(false)
        private set
    var profileAccountLoadFailed by mutableStateOf(false)
        private set
    var profileAccountRetry by mutableIntStateOf(0)
        private set
    var profileReloadRequest by mutableIntStateOf(0)
        private set
    private val followStates = mutableStateMapOf<String, String>()
    private val followRevisions = mutableMapOf<String, Int>()
    private val pendingFollows = mutableStateMapOf<String, Boolean>()
    val followBusy get() = profile.text("id") in pendingFollows
    val followResponses = mutableStateMapOf<String, String>()
    val followState: String? get() = followStates[profile.text("id")]
    var federatePost by mutableStateOf(false)
        private set
    var federationAvailable by mutableStateOf(false)
        private set
    var query by mutableStateOf("")
    /** The `/search` field; the controller searches it after typing pauses. */
    var searchText by mutableStateOf(app.globalSearch.query)
    var postSelection by mutableStateOf<TextRange?>(null)
    var quoteDraft by mutableStateOf<TimelineItem?>(null)
    var postDraft by mutableStateOf("")
    var replyDraft by mutableStateOf("")
    val bubble = BubbleController(app)
    var leaveTarget by mutableStateOf<Pair<String, String?>?>(null)
    var photosOpen by mutableStateOf(false)
    var recent by mutableStateOf<List<RecentPhoto>>(emptyList())
    var postDetail by mutableStateOf<List<Row>>(emptyList())
    var saving by mutableStateOf(false)
        private set
    var channelSaveError by mutableStateOf<String?>(null)
    var channelName by mutableStateOf("")
    var channelDescription by mutableStateOf("")
    var channelPrivate by mutableStateOf(false)
        private set
    var channelSensitive by mutableStateOf(false)
        private set
    var channelEncrypted by mutableStateOf(false)
        private set
    var postVisibility by mutableStateOf("unlisted")
    /** Serializes notification-level PATCHes per channel so the last selection is saved last. */
    private val notificationLevelLocks = mutableMapOf<String, Mutex>()
    private val drafts = mutableStateMapOf<String, InputState>()

    fun collapseInput(channelId: String?, threadId: String?): Boolean {
        val key = channelId?.let { it + (threadId?.let { ":$it" } ?: "") } ?: return false
        val draft = drafts[key]?.takeIf { it.expanded } ?: return false
        drafts[key] = draft.copy(expanded = false)
        return true
    }

    private fun AttachmentDrafts.maxImageSize() {
        acceptedTypes = imageTypes
        maximumBytes = 8L * 1024 * 1024
        enabled = true
    }

    var linkEmojiDemandCount by mutableIntStateOf(0)
        private set

    var linkEmojiDemandVersion by mutableLongStateOf(0L)
        private set

    fun setLinkEmojiDemand(active: Boolean) {
        linkEmojiDemandCount += if (active) 1 else -1
        if (active) linkEmojiDemandVersion++
    }

    val workspaceEmojiIndex by derivedStateOf {
        workspaceEmojiImages(app.config.apiBaseUrl, app.workspaceEmojis)
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
                displayEmoji(original, original, original.text("content"), isPost = true),
                avatar = timelineAvatarUrl(app.config.apiBaseUrl, original, users[original.text("user_id")]),
                postAuthor = postAuthor(original),
                local = quoteRow.flag("local"),
                handle = quoteHandle(original.text("user_name"), if (original.flag("remote")) original.text("user_id") else uri),
                remotePostAvailable = original.flag("remote") && original.text("id").isNotBlank(),
            )
        }
        val body = data.text("content").let { content ->
            val rawQuoteUrl = quoteRow.text("uri")
            if (quote == null) content
            else content.trimEnd().removeSuffix("\n\nRE: <$rawQuoteUrl>").removeSuffix("<$rawQuoteUrl>").removeSuffix(rawQuoteUrl).trimEnd()
        }
        val richContent = if (!isPost && !data.present("ciphertext")) messageRichContent(data) else EntryRichContent()
        val metadata = display.ifEmpty { data }
        val emojiBodies = richContent.blocks.asSequence().flatMap { it.displayedText() }.toList()
        val visibleBody = if (richContent.hidesBody) "" else body
        val emoji = if (allowPreview && !data.present("ciphertext")) displayEmoji(data, metadata, visibleBody, isPost, emojiBodies) else emptyMap()
        val reactionEmoji = if (allowPreview && !data.present("ciphertext")) {
            val values = if (isPost) metadata.array("reactions").rows().map { it.text("reaction") }
            else reactions.map { it.text("emoji") }
            values.associateWith { emoji[it]?.url ?: workspaceEmojiIndex[it] }
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
            val accountName = accountEmojiName(value)
            val reactionImage = if (accountName == null) image else {
                val account = users.values.firstOrNull { it.text("user_name") == accountName }
                account?.let { timelineAvatarUrl(app.config.apiBaseUrl, it) }
            }
            TimelineReaction(value, label, rows.size.toString(), rows.any { it.text("user_id") == app.config.userId },
                reactionImage, canToggle = timelineReactionCanToggle(value, selected = false, canReact = true, post = false))
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
        if (!isPost) searchPreviewLinks.remember(data, url.takeUnless { data.present("deleted_at") })

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
            canQuoteRenote = isPost && (data.flag("apub") || data.flag("remote")) && data.text("visibility") in setOf("public", "unlisted"),
            canRepost = isPost && data.text("visibility") in setOf("public", "local", "unlisted"),
            reposted = metadata.flag("reposted"), reactions = badges, attachments = files, link = url,
            attachmentSizes = fileRows.associate { it.text("id") to it.text("file_size") },
            richContent = richContent, postLinks = links.posts, channelLinks = links.channels.map { it.first },
            needsWorkspaceEmoji = allowPreview && !data.present("ciphertext") && (
                ((!isPost || !data.flag("remote")) && (
                    hasMissingWorkspaceEmoji(visibleBody, emoji) || emojiBodies.any { hasMissingWorkspaceEmoji(it, emoji) } ||
                        badges.any { it.image == null && isWorkspaceEmojiCandidate(it.value) }
                )) || (quote != null && !original.flag("remote") && hasMissingWorkspaceEmoji(quote.body, quote.emoji))
            ),
            emoji = emoji, attachedImageUrls = mediaUrls, avatar = timelineAvatarUrl(app.config.apiBaseUrl, data, users[data.text("user_id")]),
            entryKey = HomeTimeline.key(row).ifBlank { id }, media = media, postHeader = postHeader, quote = quote,
            postAuthor = if (isPost) postAuthor(data, name) else null,
            reposterAvatar = timelineAvatarUrl(app.config.apiBaseUrl, obj("avatar_url" to data["reposter_avatar_url"],
                "avatar" to data["reposter_avatar"]), users[data.text("reposter_actor").removePrefix("local:")]),
            repostedBy = data.text("reposted_by").ifBlank { data.text("reposter_actor") }.ifBlank { null },
        ), previewService)
    }

    private fun postAuthor(post: Row, name: String = post.text("display_name")): Row? =
        post.text("user_id").takeIf { it.isNotBlank() }?.let { id ->
            obj("id" to id, "display_name" to name, "user_name" to post.text("user_name"),
                "remote" to post.flag("remote"), "avatar" to post["avatar"], "avatar_url" to post["avatar_url"])
        }

    private val operationStates = mutableStateMapOf<Pair<String, EntryOperation>, EntryOperationStatus>()

    fun operationStatus(action: AndroidUiAction.ActOnEntry) = operationStates[action.item.id to action.operation]

    fun withLinkCard(entry: TimelineItem, service: String): TimelineItem {
        val status = buildMap {
            for (operation in listOf(EntryOperation.React, EntryOperation.Repost, EntryOperation.Delete)) {
                operationStates[entry.id to operation]?.let { put(operation, it) }
            }
        }
        return entry.copy(operationStatus = status, linkCard = entry.link?.let {
            summalyCard(service, it, entry.attachedImageUrls)
        })
    }

    private fun changeEntry(action: AndroidUiAction.ActOnEntry, change: suspend () -> Unit) {
        val key = action.item.id to action.operation
        if (operationStates[key] == EntryOperationStatus.Sending) return
        if (!platform.env.online) {
            operationStates[key] = EntryOperationStatus.Offline
            return
        }
        operationStates[key] = EntryOperationStatus.Sending
        app.scope.launch {
            try {
                change()
                operationStates.remove(key)
            } catch (cancelled: CancellationException) {
                operationStates.remove(key)
                throw cancelled
            } catch (error: Exception) {
                operationStates[key] = if (error is ApiException && error.status == 403)
                    EntryOperationStatus.Denied else EntryOperationStatus.Failed
            }
        }
    }

    private fun summalyCard(service: String, url: String, attachedImages: Set<String> = emptySet()): EntryCard? {
        val preview = app.previews.cached(service, url) ?: return null
        val image = publicUrlString(preview.image)?.takeUnless { it in attachedImages }
            ?.let { TimelineMedia(it, "", TimelineMediaKind.Image, url = it) }
        return EntryCard(preview.title, preview.description, url, preview.site, image = image,
            formatted = false, summaly = true)
    }

    private fun displayEmoji(
        data: Row, metadata: Row, body: String, isPost: Boolean, displayedBlocks: List<String> = emptyList(),
    ): Map<String, TimelineMedia> =
        timelineBodyEmojis(app.config.apiBaseUrl, data, metadata, body, isPost, workspaceEmojiIndex, displayedBlocks)

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
                } else changeEntry(action) {
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
            EntryOperation.QuoteRenote -> if (post && action.item.canQuoteRenote) {
                quoteDraft = action.item
                if (action.item.postHeader?.audience == PostAudience.Unlisted && postVisibility == "public") postVisibility = "unlisted"
                overlay = "new-post"
            }
            EntryOperation.Repost -> if (post && action.item.canRepost) changeEntry(action) {
                app.api.request("/api/activitypub/reposts", if (action.item.reposted) "DELETE" else "POST",
                    obj("post_id" to action.item.id))
                refreshPost()
            }
            EntryOperation.Attachment -> attachmentAction = action
            EntryOperation.Link -> if (!encrypted && publicUrl(action.value) != null) platform.action("open:${action.value}")
            EntryOperation.Media -> if (!encrypted) {
                val media = if (action.quotedMedia) action.item.quote?.media.orEmpty() else action.item.media
                selectMediaPreview(media, action.value)?.let {
                    mediaPreview = it
                }
            }
            EntryOperation.Quote -> action.item.quote?.let { quote ->
                val postId = quote.postId
                if (postId != null) openPost(postId)
                else if (quote.canOpen) platform.action("open:${quote.uri}")
            }
        }
    }

    var attachmentAction by mutableStateOf<AndroidUiAction.ActOnEntry?>(null)

    fun exportAttachment(share: Boolean) {
        val action = attachmentAction ?: return
        attachmentAction = null
        action.item.encryptedFiles[action.value]?.let { openEncryptedAttachment(it, share); return }
        val name = action.item.attachments.firstOrNull { it.first == action.value }?.second ?: return
        request {
            val bytes = app.api.getBytes(app.attachmentLinks.get(action.value))
            val type = attachmentType(name, "") ?: "application/octet-stream"
            if (share) platform.shareFile(bytes, name, type) else platform.saveFile(bytes, name, type)
        }
    }

    fun confirmDelete(post: Boolean) {
        val action = confirmingEntry?.takeIf { it.operation == EntryOperation.Delete && it.item.editable } ?: return
        val deletedChannelId = app.channelId
        changeEntry(action) {
            val endpoint = if (post) "home" else "messages"
            app.api.request("/api/$endpoint/${action.item.id.encodeURLParameter()}", "DELETE")
            if (confirmingEntry == action) confirmingEntry = null
            if (!post) {
                searchPreviewLinks.removeMessage(action.item.id)
                deletedChannelId?.let { app.globalSearch.removeDeletedMessage(it, action.item.id) }
            }
            if (post) {
                val deletedId = action.item.id
                discardPost(deletedId)
                if (openedPostId == deletedId) {
                    if (overlay == "post") closePost() else clearPost()
                }
                refreshPost()
            } else if (app.thread == action.item.id) app.thread = null
        }
    }

    private fun discardPost(id: String) {
        postRevision++
        mediaPreview = null
        home.remove(id)
        feedBackStack.removeAll { it is FeedOrigin.Post && it.id == id }
        profilePosts = profilePosts.mapNotNull { it.withoutUnavailableHomePost(id) }
        postDetail = postDetail.mapNotNull { it.withoutUnavailableHomePost(id) }
    }

    private suspend fun loadProfilePosts(id: String): List<Row> =
        app.api.request("/api/home?timeline=social&author=${id.encodeURLParameter()}&limit=40").rows()

    suspend fun reloadActiveProfile() {
        if (overlay != "profile" || !profilePostsPending) return
        val id = profile.text("id")
        val revision = postRevision
        val screen = feedScreenRevision
        fun isCurrent() = overlay == "profile" && profile.text("id") == id &&
            postRevision == revision && feedScreenRevision == screen
        profilePostsLoadFailed = false
        try {
            val posts = loadProfilePosts(id)
            if (isCurrent()) {
                profilePosts = posts
                app.cacheHomeDetail("profile:$id", posts)
                profilePostsRevision = revision
            }
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (failure: Exception) {
            if (isCurrent()) {
                if (failure is ApiException && failure.status in setOf(403, 404)) {
                    app.removeCachedHomeDetail("profile:$id")
                    profilePosts = emptyList()
                }
                profilePostsLoadFailed = true
            }
        }
    }

    private suspend fun refreshOpenPost(id: String) {
        val revision = postRevision
        val screen = feedScreenRevision
        fun isCurrent() = overlay == "post" && openedPostId == id &&
            postRevision == revision && feedScreenRevision == screen
        val detail = try {
            app.api.request("/api/home/${id.encodeURLParameter()}/thread").rows()
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (failure: Exception) {
            if (isCurrent()) {
                if (failure is ApiException && failure.status in setOf(403, 404)) {
                    closeUnavailablePost(id, failure.status)
                    return
                }
            }
            throw failure
        }
        if (isCurrent()) {
            if (detail.isEmpty()) {
                closeUnavailablePost(id)
            } else {
                postDetail = detail
                app.cacheHomeDetail("post:$id", detail)
                postDetailRevision = revision
            }
        }
    }

    private fun closeUnavailablePost(id: String, status: Int? = null) {
        app.toast = app.t(when (status) {
            403 -> "common.errors.forbidden"
            404 -> "common.errors.notFound"
            else -> "home.postUnavailable"
        })
        discardPost(id)
        closePost()
        home.refresh(retain = true)
    }

    suspend fun reloadActivePost() {
        val id = openedPostId ?: return
        if (overlay != "post" || postDetailRevision == postRevision) return
        postLoadFailed = false
        try {
            refreshOpenPost(id)
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (_: Exception) {
            if (overlay == "post" && openedPostId == id) postLoadFailed = true
        }
    }

    private fun refreshPost() {
        postRevision++
        home.refresh(retain = true)
    }

    private fun revalidateAfterUnfollow() {
        postRevision++
        profilePosts = emptyList()
        postDetail = emptyList()
        postDetailRevision = -1
        mediaPreview = null
        if (overlay == "profile" || overlay == "post") confirmingEntry = null
        home.reloadAfterAudienceChange()
    }

    suspend fun searchPeople(term: String) {
        people = if (term.isBlank()) emptyList()
        else app.api.request("/api/activitypub/search?q=${term.encodeURLParameter()}").rows()
    }

    suspend fun loadFederation() {
        federationAvailable = app.api.request("/api/activitypub/status").jsonObject.flag("enabled")
    }

    private fun openProfile(person: Row) {
        val id = person.text("id")
        val source = overlay
        if (id.isBlank() || (source == "profile" && profile.text("id") == id)) return
        rememberFeedOrigin(source)
        clearPost()
        profilePostsLoadFailed = false
        profile = person
        profileAccountLoadFailed = false
        profilePosts = app.cachedHomeDetail("profile:$id")
        profilePostsRevision = -1
        overlay = "profile"
        feedScreenRevision++
    }

    suspend fun reloadProfileAccount() {
        if (overlay != "profile") return
        val person = profile
        val id = person.text("id")
        val screen = feedScreenRevision
        val followRevision = followRevisions[id]
        fun isCurrent() = overlay == "profile" && feedScreenRevision == screen && profile.text("id") == id
        profileAccountLoadFailed = false
        val account = try {
            app.api.request("/api/activitypub/accounts?actor=${id.encodeURLParameter()}").jsonObject
        } catch (cancelled: CancellationException) { throw cancelled }
        catch (failure: Exception) {
            if (failure is ApiException && failure.status in setOf(403, 404)) {
                app.removeCachedHomeDetail("profile:$id")
                followStates.remove(id)
                feedBackStack.removeAll { it is FeedOrigin.Profile && it.account.text("id") == id }
                if (isCurrent()) {
                    profile = obj()
                    profilePosts = emptyList()
                    closeProfile()
                    app.toast = app.t("common.errors.notFound")
                }
            } else if (isCurrent()) {
                profileAccountLoadFailed = true
            }
            return
        }
        if (!isCurrent()) return
        profile = JsonObject(person + account)
        if (id !in pendingFollows && followRevisions[id] == followRevision) {
            if (!profile.flag("remote") || id == app.config.userId) followStates.remove(id)
            else followStates[id] = when {
                !account.present("following") -> "フォロー"
                account.flag("following") -> "フォローを解除"
                else -> "申請を取り消す"
            }
        }
    }

    fun closeProfile() {
        restoreFeedOrigin()
    }

    private fun toggleProfileFollow() {
        val id = profile.text("id")
        val state = followStates[id] ?: return
        if (!platform.env.online || id in pendingFollows) return
        pendingFollows[id] = true
        val actor = profile.text("actor_id").ifBlank { id }
        val revision = (followRevisions[id] ?: 0) + 1
        followRevisions[id] = revision
        request {
            try {
                if (state == "フォロー") {
                    app.api.request("/api/activitypub/follows", "POST", obj("actor" to actor))
                    followStates[id] = updateFollowState(actor)
                } else {
                    app.api.request("/api/activitypub/follows", "DELETE", obj("actor" to actor))
                    followStates[id] = "フォロー"
                    revalidateAfterUnfollow()
                }
            } finally {
                pendingFollows.remove(id)
                followRevisions[id] = revision + 1
            }
        }
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
        val mention = mentionRange(draft)
        return draft.copy(
            mentions = zoo.completionOptions(mention, people, people.map { it.text("id") }.toSet()),
            canSend = canPost && !saving && (files == null || files.canSend(draft.text)),
            hasAttachments = files?.items?.isNotEmpty() == true,
            thumbnails = files?.items.orEmpty().mapNotNull { item -> item.bitmap?.let { item.file.text("id") to it } }.toMap(),
            failedFiles = files?.items?.filter { it.failed }?.map { it.file.text("id") }?.toSet().orEmpty(),
            files = files?.items?.map { it.file.text("id") to it.name }.orEmpty(),
        )
    }

    private fun mentionRange(draft: InputState): zoo.CompletionRange? {
        val selection = draft.selection
        if (selection != null && !selection.collapsed) return null
        return zoo.completionAt(draft.text, selection?.end ?: draft.text.length)?.takeIf { it.kind == "mention" }
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
        val active = channels.filter {
            it.text("id") in joined && !it.present("deleted_at") && !it.present("archive_at")
        }.sortedWith { left, right ->
            compareLocaleText(left.text("channel_name"), right.text("channel_name"), app.env.locale)
        }
        fun item(row: Row): ChannelItem {
            val id = row.text("id")
            val peers = members.filter { it.text("channel_id") == id && it.text("user_id") != app.config.userId }
            val userName = peers.singleOrNull()?.let { users[it.text("user_id")] }?.let(::author)
                ?.takeIf { row.flag("is_dm") && it.isNotBlank() }
            return ChannelItem(id, userName ?: row.text("channel_name"), app.channelReadPositions[id]?.hasUnread == true, userName != null, direct = row.flag("is_dm"))
        }
        val activeById = active.associateBy { it.text("id") }
        val unpinned = active.filterNot { it.text("id") in pins }
        return listOf(
            ChannelSection("ピン留め", pins.mapNotNull(activeById::get).map(::item)),
            ChannelSection("参加中", unpinned.filterNot { it.flag("is_dm") }.map(::item)),
            ChannelSection("ダイレクトメッセージ", unpinned.filter { it.flag("is_dm") }.map(::item)),
        )
    }

    private val searchPreviewLinks = SearchPreviewLinks()

    fun forgetSearchPreviews(rows: List<Row>) {
        rows.forEach { row ->
            if (row.present("deleted_at") || row.present("ciphertext")) {
                searchPreviewLinks.remove(row.text("channel_id"), row.text("id"))
                if (row.present("deleted_at")) app.globalSearch.removeDeletedMessage(row.text("channel_id"), row.text("id"))
            }
        }
    }

    fun searchScreen(
        users: Map<String, Row>, channels: List<Row>, members: List<Row>, previewService: String,
    ): AndroidScreen.Search {
        val search = app.globalSearch
        val channelsById = channels.associateBy { it.text("id") }
        val memberChannels = searchMemberChannels(members)
        val guest = isSearchGuest(users[app.config.userId])
        val messagesByChannel = mutableMapOf<String, Map<String, Row>>()
        val year = platform.day(platform.now()).substringBefore('/')
        return AndroidScreen.Search(SearchScreenState(
            searchText, search.query, search.type,
            search.items.mapNotNull { row ->
                val channelId = row.text("channel_id")
                val channel = channelsById[channelId]?.takeIf {
                    canViewSearchChannel(it, channelId in memberChannels, guest)
                } ?: return@mapNotNull null
                val file = isGlobalSearchFile(row, search.type)
                val searchAuthor = row.nested("author")
                val user = users[row.text("user_id")] ?: searchAuthor
                val link = if (file || row.text("kind") == "note") null else {
                    val current = messagesByChannel.getOrPut(row.text("channel_id")) {
                        app.repo.cachedMessages(row.text("channel_id")).associateBy { it.text("id") }
                    }[row.text("id")]
                    searchPreviewLinks.cached(row, current)
                }
                val createdAt = row.text("created_at")
                val day = platform.day(createdAt).let { if (it.substringBefore('/') == year) it.substringAfter('/') else it }
                // Like Web, keep the first hit within the three lines of a narrow screen.
                val body = if (file) row.text("file_name") else focusSnippet(row.text("snippet"), searchTerms(search.query))
                val emoji = if (file) emptyMap() else displayEmoji(row, row, body, isPost = false)
                SearchResultItem(
                    key = globalSearchKey(row, search.type),
                    author = author(user).ifBlank { row.text("user_id") },
                    body = body,
                    time = listOf(day, platform.time(createdAt)).filter { it.isNotBlank() }.joinToString(" "),
                    destination = searchDestination(row, channel, users, members),
                    dm = isDm(row, channel),
                    inThread = row.text("parent_id").isNotEmpty(),
                    file = file,
                    fileSize = if (file) attachmentSize(row.text("file_size")) else null,
                    emoji = emoji,
                    needsWorkspaceEmoji = !file && hasMissingWorkspaceEmoji(body, emoji),
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

    private fun searchMemberChannels(members: List<Row>): Set<String> =
        members.asSequence().filter { it.text("user_id") == app.config.userId && !it.present("deleted_at") }
            .map { it.text("channel_id") }.toSet()

    private fun isSearchGuest(user: Row?): Boolean = user == null || user.flag("is_guest")

    private fun canViewSearchChannel(channel: Row, member: Boolean, guest: Boolean): Boolean =
        searchChannelVisible(channel, member, guest, app.isEncryptedChannel(channel.text("id")))

    private fun currentSearchChannel(id: String): Row? {
        val members = searchMemberChannels(app.repo.members.rows.value)
        val guest = isSearchGuest(app.repo.users.rows.value.firstOrNull { it.text("id") == app.config.userId })
        return app.repo.channels.rows.value.firstOrNull { it.text("id") == id }
            ?.takeIf { canViewSearchChannel(it, id in members, guest) }
    }

    fun canKeepMediaPreview(selection: TimelineMediaSelection): Boolean {
        val target = selection.searchTarget ?: return true
        val channel = currentSearchChannel(target.channelId) ?: return false
        if (channel.flag("is_nsfw") && !selection.selected.sensitive) return false
        val message = target.messageId?.let { app.repo.linkedMessages.peek(target.channelId, it) }
        return message == null || !searchParentUnavailable(message)
    }

    fun dismissUnavailableSearchPreview(selection: TimelineMediaSelection, message: LinkedMessageState) {
        if (mediaPreview != selection || !searchParentUnavailable(message)) return
        val target = selection.searchTarget ?: return
        val parent = message.row
        if (parent is LinkedRow.Missing || (parent is LinkedRow.Found && parent.row.present("deleted_at"))) {
            target.messageId?.let { app.globalSearch.removeDeletedMessage(target.channelId, it) }
        }
        mediaPreview = null
    }

    private fun searchParentUnavailable(message: LinkedMessageState): Boolean =
        message.hasCiphertext || when (val parent = message.row) {
            LinkedRow.Missing, LinkedRow.Failed -> true
            is LinkedRow.Found -> parent.row.present("deleted_at") || parent.row.present("ciphertext")
            LinkedRow.Loading -> false
        }

    private var searchMediaRequest = 0

    private fun openSearchMedia(action: AndroidUiAction.OpenSearchMedia) {
        val search = app.globalSearch
        val row = search.items.firstOrNull {
            isGlobalSearchFile(it, search.type) && globalSearchKey(it, search.type) == action.key
        } ?: return
        val id = row.text("channel_id")
        val parent = row.text("message_id").takeIf { it.isNotBlank() } ?: return
        val media = currentSearchChannel(id)?.let { searchImage(row, it) } ?: return
        if (media.sensitive && !action.revealedSensitive) return
        val attachmentId = media.attachmentId ?: return
        val selection = TimelineMediaSelection(listOf(media), 0, ChannelLinkTarget(id, parent))
        if (!canKeepMediaPreview(selection)) return
        val requestId = ++searchMediaRequest
        val navigation = platform.navigationRevision
        request {
            // Cached signed URLs can outlive deletion. Ask the attachment API to authorize this open.
            try {
                app.attachmentLinks.get(attachmentId, force = true)
            } catch (failure: ApiException) {
                if (failure.status !in setOf(403, 404, 410)) throw failure
                return@request
            }
            if (requestId == searchMediaRequest && navigation == platform.navigationRevision &&
                platform.env.visible && row in search.items && canKeepMediaPreview(selection)) {
                mediaPreview = selection
            }
        }
    }

    private fun searchImage(row: Row, channel: Row): TimelineMedia? =
        timelineAttachmentMedia(row)?.takeIf { it.kind == TimelineMediaKind.Image }
            ?.let { it.copy(sensitive = it.sensitive || channel.flag("is_nsfw")) }

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

    fun channelSettings(channel: Row, member: Boolean, encrypted: Boolean, notificationLevel: String?): List<SettingsGroup> {
        val archived = channel.present("archive_at")
        val entries = buildList {
            if (channel.text("description").isNotBlank()) add(SettingsEntry.Information("description", channel.text("description")))
            if (encrypted) add(SettingsEntry.Information("encrypted", "暗号化チャンネル"))
            add(SettingsEntry.Destination("members", "メンバー"))
            if (member && !archived) {
                val pinned = channel.text("id") in pins
                add(SettingsEntry.Destination(if (pinned) "unpin" else "pin", if (pinned) "ピン留めを解除" else "ピン留め"))
            }
            if (canEditMembers(channel)) add(SettingsEntry.Destination("add-member", "メンバーを追加"))
            if (canManage(channel, app.config.userId, app.config.isAdmin) && !channel.flag("is_dm")) {
                add(SettingsEntry.Destination(if (archived) "unarchive" else "archive",
                    if (archived) "アーカイブを解除" else "アーカイブ"))
            }
            if (canManage(channel, app.config.userId, app.config.isAdmin) &&
                !archived && !channel.flag("is_dm")) {
                add(SettingsEntry.Destination("rename", "名前と説明を編集"))
            }
            if (!channel.flag("is_dm") && !archived) {
                if (member) add(SettingsEntry.Destination("leave", "退出"))
                else if (!channel.flag("is_private")) add(SettingsEntry.Destination("join", "参加"))
            }
        }
        val groups = mutableListOf(SettingsGroup("チャンネル", entries))
        // 参加中だけ設定できる。値は channel_notification_settings の Shape の行から渡され、
        // 書き込み後も同じ行の更新で選択状態が変わる（設定ごとの REST 取得は増やさない）。
        // 行が届く前は現在値が分からないので、誤って「メンション」が選択済みに見えないよう
        // 選択肢を出さず「読み込み中」を示す（Web と同じ）。
        if (member && !archived) {
            val notificationEntries = notificationLevel?.let { level ->
                listOf(
                    SettingsEntry.Choice("notification:all", "すべての投稿", level == "all", "通常のチャンネル投稿も届きます"),
                    SettingsEntry.Choice("notification:mentions", "メンション", level == "mentions", "自分への返信やリアクションも届きます"),
                    SettingsEntry.Choice("notification:mute", "メンション以外を通知しない", level == "mute", "自分への明示的なメンションだけ届きます"),
                )
            } ?: listOf(
                SettingsEntry.Information(
                    "notification:loading",
                    app.t("notifications.levelLoading")
                )
            )
            groups += SettingsGroup("通知", notificationEntries)
        }
        return groups
    }

    fun settings(env: Environment, fullScreenAllowed: Boolean?): List<SettingsGroup> = listOf(
        SettingsGroup("外観", listOf(SettingsEntry.Destination("theme", "テーマ", env.theme),
            SettingsEntry.Destination("language", "言語", env.locale))),
        SettingsGroup("操作", listOf(SettingsEntry.Toggle("swipe", "スワイプで返信", env.swipeReply),
            SettingsEntry.Toggle("haptics", "触覚フィードバック", env.operationHaptics))),
        SettingsGroup("Zoo", listOf(SettingsEntry.Destination("search-engine", "検索エンジン", app.searchEngine),
            SettingsEntry.Destination("notifications", "通知"),
            SettingsEntry.Destination("navigation-cache", app.t("androidNavigationCache.title"), cacheLimitLabel(platform.navigationCacheLimit)),
            SettingsEntry.Destination("app-update", "アプリの更新"),
            SettingsEntry.Destination("encryption", "エンドツーエンド暗号化"),
            SettingsEntry.Destination("logout", "ログアウト"))),
        SettingsGroup("通話", buildList {
            add(SettingsEntry.Destination("sip", app.t("sip.title")))
            add(SettingsEntry.Toggle("dm-calls", "DMの着信を受ける", phone.dmWaiting))
            if (phone.dmWaitingError) add(SettingsEntry.Information("dm-call-error", app.t("androidPhone.callError")))
            if (fullScreenAllowed != null) {
                add(SettingsEntry.Destination("incoming-screen", app.t("androidPhone.incomingScreenSettings"), if (fullScreenAllowed) "許可" else "許可が必要"))
            }
        }),
    )

    private fun cacheLimitLabel(limit: Int) = if (limit == 0) app.t("androidNavigationCache.off")
        else app.t("androidNavigationCache.entries", "count" to limit.toString())

    fun settingsDetail(env: Environment, notificationOptions: List<SettingsEntry>): AndroidScreen {
        val (title, entries) = when (overlay) {
            "theme" -> "テーマ" to listOf("system" to "端末に従う", "light" to "ライト", "dark" to "ダーク")
                .map { SettingsEntry.Destination("theme:${it.first}", it.second, if (env.theme == it.first) "選択中" else null) }
            "language" -> "言語" to listOf("ja" to "日本語", "en" to "English", "ko-KR" to "한국어", "ko-KP" to "조선말",
                "zh-Hans" to "简体中文", "zh-Hant" to "繁體中文", "yue" to "粵語").map {
                SettingsEntry.Destination("locale:${it.first}", it.second, if (env.locale == it.first) "選択中" else null)
            }
            "navigation-cache" -> app.t("androidNavigationCache.title") to buildList<SettingsEntry> {
                add(SettingsEntry.Information("navigation-cache-info", app.t("androidNavigationCache.description") + " 件数を減らすと、古い画面に戻った際に再読み込みします。"))
                add(SettingsEntry.Destination("clear-navigation-cache", "画面キャッシュを消去", "ホームと過去に開いた画面の保存内容"))
                navigationCacheLimits.forEach { limit ->
                    add(SettingsEntry.Choice("navigation-cache:$limit", cacheLimitLabel(limit),
                        platform.navigationCacheLimit == limit))
                }
            }
            "search-engine" -> "検索エンジン" to searchEngines.map {
                SettingsEntry.Destination("engine:${it.key}", it.value, if (app.searchEngine == it.key) "選択中" else null)
            }
            else -> "通知" to notificationOptions
        }
        val groupTitle = if (overlay == "navigation-cache") "" else title
        return AndroidScreen.SettingsDetail(title, listOf(SettingsGroup(groupTitle, entries)))
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
            is AndroidUiAction.RespondFollow -> if (followResponses[action.activity] in listOf(null, "再試行できます")) request {
                if (followResponses[action.activity] !in listOf(null, "再試行できます")) return@request
                followResponses[action.activity] = "処理中"
                try {
                    app.api.request("/api/activitypub/follows/respond", "POST",
                        obj("actor" to action.actor, "activity_id" to action.activity, "accept" to action.accept))
                    followResponses[action.activity] = if (action.accept) "承認済み" else "拒否済み"
                    app.activityPubNotifications.refresh()
                } catch (e: CancellationException) { throw e }
                catch (_: Exception) { followResponses[action.activity] = "再試行できます" }
            }
            is AndroidUiAction.SelectInput -> when (screen) {
                is AndroidScreen.EditEntry -> editSelection = action.range
                is AndroidScreen.NewPost -> postSelection = action.range
                is AndroidScreen.Post -> replySelection = action.range
                is AndroidScreen.Conversation, is AndroidScreen.Thread ->
                    drafts[draftKey] = (drafts[draftKey] ?: InputState()).copy(selection = action.range)
                else -> Unit
            }
            is AndroidUiAction.SetPhotoSensitive -> if (screen is AndroidScreen.NewPost) {
                postFiles.items.find { it.file.text("id") == action.id }?.sensitive = action.sensitive
            }
            is AndroidUiAction.DescribePhoto -> if (screen is AndroidScreen.NewPost && action.description.codePointCount(0, action.description.length) <= 1000) {
                postFiles.items.find { it.file.text("id") == action.id }?.description = action.description
            }
            is AndroidUiAction.RetryFile -> (if (screen is AndroidScreen.NewPost) postFiles else channelFiles[draftKey])?.retry(action.id)
            is AndroidUiAction.RemoveFile -> {
                val attachments = if (screen is AndroidScreen.NewPost) postFiles else channelFiles[draftKey]
                attachments?.items?.firstOrNull { it.file.text("id") == action.id }?.let { attachments.remove(it) }
            }
            is AndroidUiAction.Mention -> {
                val draft = drafts[draftKey] ?: InputState()
                mentionRange(draft)?.let { range ->
                    val (text, caret) = zoo.insertCompletion(draft.text, range, action.name)
                    drafts[draftKey] = draft.copy(text = text, selection = TextRange(caret))
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
            AndroidUiAction.RetryPost -> postReloadRequest++
            AndroidUiAction.RetryProfile -> profileReloadRequest++
            AndroidUiAction.RetryProfileAccount -> profileAccountRetry++
            AndroidUiAction.RetryTimeline -> if (home.olderError) home.older()
                else home.refresh(retain = home.posts.isNotEmpty())
            is AndroidUiAction.OpenProfile -> openProfile(action.author)
            is AndroidUiAction.ReplyToPost -> openPost(action.id, focusReply = true)
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
            is AndroidUiAction.OpenSearchMedia -> openSearchMedia(action)
            AndroidUiAction.LoadOlderResults -> app.globalSearch.loadOlder()
            AndroidUiAction.RetrySearch -> app.globalSearch.retry()
            is AndroidUiAction.SelectResult -> when (overlay) {
                "people" -> people.firstOrNull { it.text("id") == action.id }?.let(::openProfile)
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
                action.id == "profile-follow" -> toggleProfileFollow()
                action.id == "submit-dm" && dmTargets.isNotEmpty() && !saving -> request {
                    saving = true
                    try {
                        clearSyncedDms(app.repo.channels.rows.value, app.repo.members.rows.value)
                        val selection = DmSelection(dmTargets, encryptedDm)
                        val channel = pendingDms[selection] ?: app.createDm(selection.members, selection.encrypted).also {
                            pendingDms[selection] = it
                        }
                        overlay = null
                        platform.navigate("/channels/${channel.text("id").encodeURLParameter()}")
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
                action.id == "leave" && channel != null -> leaveTarget = channelId to null
                action.id == "thread-leave" && channel != null && app.thread != null -> leaveTarget = channelId to app.thread
                action.id in setOf("thread-join", "thread-unarchive") ->
                    app.thread?.let { id -> request { loadThread(id, action.id.removePrefix("thread-")) } }
                action.id in setOf("pin", "unpin") && channel != null -> request {
                    if (action.id == "pin") app.api.request("/api/channel-pins", "POST", obj("channel_id" to channelId))
                    else app.api.request("/api/channel-pins/${channelId.encodeURLParameter()}", "DELETE")
                    refreshPins()
                }
                action.id.startsWith("notification:") && channel != null -> {
                    val level = action.id.substringAfter(':')
                    // 設定行が届く前に現在値を確定できないので、書き込みもしない（Web と同じ）。
                    if (level in setOf("all", "mentions", "mute") &&
                        channelNotificationLevel(app.repo.notificationSettings.rows.value, channelId) != null
                    ) {
                        if (!platform.env.online) {
                            // request() はオフラインだと PATCH を送らず黙って戻るので、
                            // 選択が保存されなかったことを利用者に伝える（共通の通信エラー表示）。
                            app.toast = app.t("common.errors.network")
                        } else {
                            val lock = notificationLevelLocks.getOrPut(channelId) { Mutex() }
                            request {
                                lock.withLock {
                                    app.api.request(
                                        "/api/channels/${channelId.encodeURLParameter()}/notification-settings",
                                        "PATCH", obj("level" to level))
                                }
                            }
                        }
                    }
                }
                action.id == "os-notifications" -> enableNotifications()
                action.id == "logout" -> request { platform.invoke("logout") }
                action.id.startsWith("theme:") || action.id.startsWith("locale:") -> platform.action(action.id)
                action.id == "clear-navigation-cache" -> {
                    app.clearNavigationCache()
                    app.toast = "画面キャッシュを消去しました"
                }
                action.id.startsWith("navigation-cache:") ->
                    action.id.substringAfter(':').toIntOrNull()?.takeIf { it in navigationCacheLimits }?.let { limit ->
                        platform.saveNavigationCacheLimit(limit)
                        app.setNavigationCacheLimit(limit)
                    }
                action.id.startsWith("engine:") -> app.searchPreferences.save(action.id.substringAfter(':'))
                action.id == "rename" && channel != null -> {
                    channelSaveError = null
                    channelName = channel.text("channel_name")
                    channelDescription = channel.text("description")
                    channelPrivate = channel.flag("is_private")
                    channelSensitive = channel.flag("is_nsfw")
                    channelEncrypted = app.isEncryptedChannel(channelId)
                    overlay = "rename"
                }
                action.id == "join" && channel != null -> request {
                    app.api.request("/api/channels/${channelId.encodeURLParameter()}/${action.id}", "POST")
                    overlay = null
                }
                else -> { query = ""; overlay = action.id }
            }
            is AndroidUiAction.ChannelNameChanged -> channelName = action.name
            is AndroidUiAction.ChannelDescriptionChanged -> channelDescription = action.description
            AndroidUiAction.SubmitChannel -> if (!saving) request {
                channelSaveError = null
                saving = true
                try {
                    val payload = obj("channel_name" to channelName.trim(), "description" to channelDescription,
                        "is_private" to channelPrivate, "is_nsfw" to channelSensitive)
                    if (overlay == "rename" && channel != null) {
                        app.repo.channels.write(app.api, "/api/channels/${channelId.encodeURLParameter()}", "PATCH", payload)
                    } else app.createChannel(kotlinx.serialization.json.JsonObject(payload + ("encrypted" to JsonPrimitive(channelEncrypted))))
                    overlay = null
                } catch (e: CancellationException) { throw e }
                catch (_: Exception) { channelSaveError = "保存できませんでした。接続を確認して、もう一度保存してください。" }
                finally { saving = false }
            }
            AndroidUiAction.Browse -> { query = ""; overlay = "browse" }
            AndroidUiAction.CreateChannel -> {
                channelSaveError = null
                channelName = ""
                channelDescription = ""
                channelPrivate = false
                channelSensitive = false
                channelEncrypted = false
                overlay = "create"
            }
            AndroidUiAction.CreateDm -> { query = ""; dmTargets = emptySet(); encryptedDm = false; overlay = "dm" }
            AndroidUiAction.RemoveQuote -> quoteDraft = null
            AndroidUiAction.ComposePost -> overlay = "new-post"
            AndroidUiAction.SystemPhotos -> postFiles.pick(photos = true)
            is AndroidUiAction.SelectPhoto -> request { postFiles.add(platform.invoke("pickRecentPhoto", action.id)) }
            is AndroidUiAction.SelectTimeline -> if (action.id != home.timeline) {
                home.select(action.id)
            }
            AndroidUiAction.LoadOlderPosts -> home.older()
            AndroidUiAction.LoadOlderMessages -> if (channelId.isNotBlank()) app.scope.launch {
                app.repo.messages(channelId).loadOlder(app.thread?.let(PageScope::Replies) ?: PageScope.Main)
            }
            is AndroidUiAction.SelectPostVisibility -> if (quoteDraft?.postHeader?.audience != PostAudience.Unlisted || action.id != "public") postVisibility = action.id
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

    fun leave(currentChannel: String?, currentThread: String?) {
        val target = leaveTarget ?: return
        leaveTarget = null
        if (target.first != currentChannel || target.second != null && target.second != currentThread) return
        request {
            val thread = target.second
            if (thread == null) {
                app.api.request("/api/channels/${target.first.encodeURLParameter()}/leave", "POST")
                overlay = null
                platform.navigate("/")
            } else {
                loadThread(thread, "leave")
            }
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

    private data class Outgoing(val destination: String, val entry: TimelineItem)
    private val outgoing = mutableStateListOf<Outgoing>()
    private val sendLocks = mutableMapOf<String, Mutex>()

    fun withOutgoing(key: String, entries: List<TimelineItem>, newestFirst: Boolean = false): List<TimelineItem> {
        val pending = outgoing.filter { it.destination == key }.map { it.entry }
        if (pending.isEmpty()) return entries
        val ids = pending.map { it.id }.toSet()
        val confirmed = entries.filterNot { it.id in ids }
        return if (newestFirst) pending.asReversed() + confirmed else confirmed + pending
    }

    private fun queueSend(homePost: Boolean, channelId: String, key: String) {
        if (!platform.env.online) return
        val destination = if (homePost) "/home" else key
        val files = if (homePost) postFiles else channelFiles[key]
        val text = if (homePost) postDraft else drafts[key]?.text.orEmpty()
        if (files?.canSend(text) == false || (text.isBlank() && files?.items.isNullOrEmpty())) return
        val sent = files?.items.orEmpty().toList()
        val uploaded = sent.map { requireNotNull(it.uploaded) }
        val parent = app.thread
        val quoted = quoteDraft.takeIf { homePost }
        val visibility = postVisibility
        val federate = federatePost && federationAvailable
        val id = platform.uuid()
        val entry = TimelineItem(id, "", listOf(text, sent.joinToString("、") { it.name })
            .filter { it.isNotBlank() }.joinToString("\n"), "", delivery = "送信中…")
        val releaseSubmission = files?.takeIf { sent.isNotEmpty() }?.retainForSubmission()
        val pending = Outgoing(destination, entry)
        outgoing.add(pending)
        files?.items?.clear()
        if (homePost) {
            quoteDraft = null
            postDraft = ""
            postSelection = null
            overlay = null
        } else drafts[key] = InputState(lastSend = id)
        val lock = sendLocks.getOrPut(destination) { Mutex() }
        app.scope.launch {
            try {
                lock.withLock {
                    if (homePost) {
                        val descriptions = sent.mapNotNull { item ->
                            item.description.takeIf { it.isNotBlank() }?.let { item.uploaded!!.text("id") to JsonPrimitive(it) }
                        }.toMap()
                        app.api.request("/api/home", "POST", obj("content" to text, "visibility" to visibility, "quote_post_id" to quoted?.id,
                            "image_ids" to JsonArray(uploaded.map { JsonPrimitive(it.text("id")) }),
                            "image_descriptions" to JsonObject(descriptions), "apub" to federate,
                            "sensitive_image_ids" to JsonArray(sent.filter { it.sensitive }.mapNotNull { it.uploaded?.get("id") })))
                        home.refresh(retain = true)
                    } else check(app.send(channelId, text, parent,
                        attachmentIds = uploaded.filter { it["encrypted"] == null }.map { it.text("id") },
                        encryptedAttachments = uploaded.mapNotNull { it["encrypted"] as? JsonObject }, messageId = id))
                }
                files?.release(sent)
                outgoing.remove(pending)
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (_: Exception) {
                val index = outgoing.indexOf(pending)
                if (index >= 0) outgoing[index] = pending.copy(entry = entry.copy(delivery = "送信を確認できませんでした",
                    recoverDraft = {
                        val current = if (homePost) postDraft else drafts[key]?.text.orEmpty()
                        if (current.isNotEmpty() || (homePost && quoteDraft != null) || !files?.items.isNullOrEmpty()) app.toast = "編集中の下書きを先に送信してください"
                        else {
                            if (homePost) {
                                quoteDraft = quoted
                                postDraft = text
                                postVisibility = visibility
                                federatePost = federate
                                overlay = "new-post"
                            } else drafts[key] = InputState(text)
                            files?.items?.addAll(sent)
                            outgoing.removeAll { it.entry.id == id }
                        }
                    }))
            } finally {
                releaseSubmission?.invoke()
                if (outgoing.none { it.destination == destination && it.entry.delivery == "送信中…" }) sendLocks.remove(destination)
            }
        }
    }

    private fun inputTool(tool: InputTool, screen: AndroidScreen, channelId: String, key: String, rows: List<Row>) {
        if (tool == InputTool.Send && (screen is AndroidScreen.NewPost || screen is AndroidScreen.Conversation || screen is AndroidScreen.Thread)) {
            queueSend(screen is AndroidScreen.NewPost, channelId, key)
            return
        }
        when (tool) {
            InputTool.Expand -> when (screen) {
                is AndroidScreen.Conversation, is AndroidScreen.Thread -> {
                    val draft = drafts[key] ?: InputState()
                    drafts[key] = draft.copy(expanded = !draft.expanded)
                }
                else -> Unit
            }
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
                                val names = app.repo.users.rows.value.map { it.text("id") to it.text("user_name") }
                                encryption.edit(requireNotNull(row), editDraft, zoo.encryptedMentions(editDraft, names))
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
                        is AndroidScreen.Post -> {
                            val id = screen.root.id
                            val text = replyDraft
                            val visibility = replyVisibility(id) ?: return@request
                            if (text.isBlank()) return@request
                            app.api.request("/api/home", "POST", obj("content" to text,
                                "visibility" to visibility, "apub" to false, "in_reply_to" to id))
                            val sent = platform.uuid()
                            if (overlay == "post" && openedPostId == id) {
                                replyLastSend = sent
                                if (replyDraft == text) {
                                    replyDraft = ""
                                    replySelection = null
                                }
                            }
                            updateSavedPost(id) { saved ->
                                if (saved.draft == text) saved.copy(draft = "", selection = null, lastSend = sent)
                                else saved.copy(lastSend = sent)
                            }
                            refreshPost()
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

    private fun replyVisibility(id: String): String? {
        val row = postDetail.map { it.nested("post").ifEmpty { it } }.firstOrNull { it.text("id") == id }
        return when (row?.text("visibility")?.takeIf { it.isNotBlank() }) {
            null -> null
            "followers" -> "followers"
            "local" -> "local"
            else -> "unlisted"
        }
    }

    private fun updateSavedPost(id: String, update: (FeedOrigin.Post) -> FeedOrigin.Post) {
        for (index in feedBackStack.indices) {
            val saved = feedBackStack[index]
            if (saved is FeedOrigin.Post && saved.id == id) feedBackStack[index] = update(saved)
        }
    }

    private fun rememberFeedOrigin(source: String?) {
        val previous = when (source) {
            "post" -> openedPostId?.let { FeedOrigin.Post(it, replyDraft, replySelection, replyFocusRequest, replyLastSend) }
                ?: FeedOrigin.Surface(null)
            "profile" -> FeedOrigin.Profile(profile)
            else -> {
                feedBackStack.clear()
                FeedOrigin.Surface(source)
            }
        }
        feedBackStack.addLast(previous)
    }

    private fun restoreFeedOrigin() {
        feedScreenRevision++
        when (val previous = feedBackStack.removeLastOrNull()) {
            is FeedOrigin.Post -> {
                openedPostId = previous.id
                postDetail = app.cachedHomeDetail("post:${previous.id}")
                postDetailRevision = -1
                replyDraft = previous.draft
                replySelection = previous.selection
                replyFocusRequest = previous.focusRequest
                replyLastSend = previous.lastSend
                overlay = "post"
            }
            is FeedOrigin.Profile -> {
                profile = previous.account
                profileAccountLoadFailed = false
                profilePosts = app.cachedHomeDetail("profile:${previous.account.text("id")}")
                profilePostsRevision = -1
                profilePostsLoadFailed = false
                overlay = "profile"
            }
            is FeedOrigin.Surface -> overlay = previous.overlay
            null -> overlay = null
        }
    }

    private fun openPost(id: String, focusReply: Boolean = false) {
        val source = overlay
        if (source == "post" && openedPostId == id) {
            if (focusReply) replyFocusRequest++
            return
        }
        val known = sequenceOf(postDetail, profilePosts, home.posts).flatten()
            .map { it.nested("post").ifEmpty { it } }
            .flatMap { sequenceOf(it, it.nested("parent"), it.nested("quote").nested("post")) }
            .firstOrNull { it.text("id") == id }
        rememberFeedOrigin(source)
        clearPost()
        openedPostId = id
        val cached = app.cachedHomeDetail("post:$id")
        val cachedRoot = cached.firstOrNull { it.nested("post").ifEmpty { it }.text("id") == id }
        postDetail = if (known == null) cached else {
            val root = cachedRoot?.let { row -> JsonObject(row.nested("post").ifEmpty { row } + known) } ?: known
            listOf(root) + cached.filterNot { it === cachedRoot }
        }
        replyFocusRequest = if (focusReply) 1 else 0
        overlay = "post"
        feedScreenRevision++
    }

    fun closePost() {
        clearPost()
        restoreFeedOrigin()
    }

    private fun clearPost() {
        openedPostId = null
        postLoadFailed = false
        replyLastSend = null
        postDetail = emptyList()
        postDetailRevision = -1
        replyDraft = ""
        replySelection = null
        replyFocusRequest = 0
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
