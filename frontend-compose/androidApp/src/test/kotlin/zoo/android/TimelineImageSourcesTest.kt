package zoo.android

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue
import zoo.android.ui.EntryContentPanel
import zoo.android.ui.EntryContentText
import zoo.android.ui.displayedText
import zoo.android.ui.TimelineMedia
import zoo.android.ui.TimelineMediaKind
import zoo.obj

class TimelineImageSourcesTest {
    private val base = "https://zoo.example"
    private val imageId = "a".repeat(64)

    @Test fun retainsRemovalOfSelectedPostReactionsButDisablesUnsupportedReactions() {
        assertFalse(timelineReactionCanToggle(":star:", selected = false, canReact = false, post = true))
        assertTrue(timelineReactionCanToggle(":star:", selected = true, canReact = false, post = true))
        assertFalse(timelineReactionCanToggle("apub:$imageId::star:", selected = true, canReact = true, post = false))
    }

    @Test
    fun avatarAndEmojiResolveAgainstTheApiAndRemoteImagesUseTheRegisteredProxy() {
        val descriptor = obj("image_id" to imageId, "url" to "https://remote.example/emoji.png")
        val proxy = "$base/api/activitypub/emoji-images/$imageId"
        assertEquals(proxy, timelineAvatarUrl(base, obj("avatar" to descriptor)))
        assertEquals(proxy, timelineEmojiUrl(base, descriptor))
        assertEquals(":party:" to proxy, timelineReactionEmoji(base, "apub:$imageId::party:", emptyMap()))
        assertEquals("$base/api/emoji-images/party", timelineAvatarUrl(base, obj("avatar_url" to "/api/emoji-images/party")))
        assertNull(timelineImageUrl(base, "//remote.example/emoji.png"))
    }

    @Test
    fun bodyUsesJoinedMetadataBeforeWorkspaceNamesAndAliases() {
        val workspace = workspaceEmojiImages(base, listOf(
            obj("name" to "party", "url" to "/api/emoji-images/party",
                "aliases" to JsonArray(listOf(JsonPrimitive("celebrate"), JsonPrimitive("お祝い 猫")))),
            obj("name" to "unused", "url" to "/api/emoji-images/unused"),
        ))
        val row = obj("display_data" to obj("emojis" to obj(
            "party" to obj("url" to "/api/emoji-images/previous"),
        )))
        val body = ":party: :celebrate: :お祝い 猫: :unknown:"
        val resolved = timelineBodyEmojis(base, row, obj("emojis" to obj()), body, false, workspace)
        assertEquals(mapOf(
            ":party:" to "$base/api/emoji-images/previous",
            ":celebrate:" to "$base/api/emoji-images/party",
            ":お祝い 猫:" to "$base/api/emoji-images/party",
        ), resolved.mapValues { it.value.url })

        val local = timelineBodyEmojis(base, obj(), obj(), body, true, workspace)
        assertEquals("$base/api/emoji-images/party", local[":celebrate:"]?.url)
        val remote = timelineBodyEmojis(base, obj("remote" to true, "emojis" to JsonArray(listOf(
            obj("name" to ":party:", "image_id" to imageId),
        ))), obj(), body, true, workspace)
        assertEquals(mapOf(":party:" to "$base/api/activitypub/emoji-images/$imageId"), remote.mapValues { it.value.url })
    }

    @Test
    fun displayedBlocksCanResolveAnAliasAbsentFromTheFallbackBody() {
        val code = ":お祝い 猫:"
        val url = "$base/api/emoji-images/party"
        val resolved = timelineBodyEmojis(base, obj(), obj(), "Fallback text", false,
            workspace = mapOf(code to url),
            displayedBlocks = EntryContentPanel(listOf(EntryContentPanel(listOf(EntryContentText("Hello $code")))))
                .displayedText().toList())
        assertEquals(url, resolved[code]?.url)
        assertTrue(hasMissingWorkspaceEmoji("Hello $code", emptyMap()))
        assertFalse(hasMissingWorkspaceEmoji("Hello $code", resolved))
        assertFalse(hasMissingWorkspaceEmoji("Hello : :@alice:", emptyMap()))
        assertTrue(hasMissingWorkspaceEmoji(":@お祝い:", emptyMap()))
        assertFalse(hasMissingWorkspaceEmoji("`:example:` <plain>:example:</plain>", emptyMap()))
        assertFalse(hasMissingWorkspaceEmoji("```txt\n:example:\n```", emptyMap()))
    }

    @Test
    fun ignoresMissingEmojiBeyondTheImageLimit() {
        val known = (1..32).associate { index -> ":emoji$index:" to "$base/emoji/$index" }
        val body = known.keys.joinToString(" ") + " :unknown:"
        val resolved = timelineBodyEmojis(base, obj(), obj(), body, false, known)
        assertFalse(hasMissingWorkspaceEmoji(body, resolved))
        assertTrue(hasMissingWorkspaceEmoji(":unknown: $body", resolved))
    }

    @Test
    fun linkExcerptRequestsTheCatalogWhenAnUnknownAliasChangesTheSummary() {
        val code = ":猫:"
        val body = "\$[ruby **星**:猫:ねこ]"
        assertTrue(linkedExcerptNeedsWorkspaceEmoji(body, emptySet()))
        val images = timelineBodyEmojis(base, obj(), obj(), body, false, mapOf(code to "$base/emoji/cat"))
        assertFalse(linkedExcerptNeedsWorkspaceEmoji(body, images.keys))
        assertFalse(linkedExcerptNeedsWorkspaceEmoji("`$body`", emptySet()))
        assertEquals("星:猫:", zoo.messageExcerpt(body, customEmojis = images.keys))

        val adjacent = "\$[ruby **星**12:30:猫:ねこ]"
        assertTrue(linkedExcerptNeedsWorkspaceEmoji(adjacent, emptySet()))
        assertFalse(linkedExcerptNeedsWorkspaceEmoji(adjacent, images.keys))
        assertEquals("星12:30:猫:", zoo.messageExcerpt(adjacent, customEmojis = images.keys))

        val formattedAlias = ":**猫**:"
        assertTrue(linkedExcerptNeedsWorkspaceEmoji(formattedAlias, emptySet()))
        assertFalse(linkedExcerptNeedsWorkspaceEmoji(formattedAlias, setOf(formattedAlias)))
        assertFalse(linkedExcerptNeedsWorkspaceEmoji(":unknown:", emptySet()))
    }

    @Test
    fun registeredMediaUsesTheProxyEvenWhenTheOriginalIsNotHttps() {
        val media = timelinePostMedia(base, obj(
            "url" to "http://remote.example/old.png", "media_type" to "image/png", "image_id" to imageId,
        ))
        assertEquals("$base/api/activitypub/media-images/$imageId", media?.previewUrl)
        assertEquals(media?.previewUrl, media?.url)
        assertNull(timelinePostMedia(base, obj("url" to "https://remote.example/unregistered.png", "media_type" to "image/png")))
    }

    @Test
    fun localPostMediaKeepsAnOriginalAndASeparateThumbnail() {
        val path = "/api/home/images/abc-123"
        val media = timelinePostMedia(base, obj("url" to path, "media_type" to "image/png"))
        assertEquals("$base$path", media?.url)
        assertEquals("$base$path?thumbnail=true", media?.thumbnailUrl)
        assertNull(media?.previewUrl)
    }

    @Test
    fun postAndChannelImagesUseStoredDimensions() {
        val hash = "LEHV6nWB2yk8pyo0adR*.7kCMdnj"
        val post = timelinePostMedia(base, obj(
            "url" to "https://remote.example/photo.png", "media_type" to "image/png", "image_id" to imageId,
            "width" to 1200, "height" to 600, "blurhash" to hash,
        ))
        val attachment = timelineAttachmentMedia(obj(
            "id" to "123", "content_type" to "image/png", "width" to "600", "height" to "1200", "blurhash" to hash,
        ))
        assertEquals(2f, post?.imageAspectRatio)
        assertEquals(0.5f, attachment?.imageAspectRatio)
        assertEquals(hash, post?.blurhash)
        assertEquals(hash, attachment?.blurhash)
        assertNull(timelineAttachmentMedia(obj(
            "id" to "123", "content_type" to "image/png", "width" to 600, "height" to 0,
        ))?.imageAspectRatio)
        assertEquals(2f, distinctTimelineMedia(listOf(
            checkNotNull(post).copy(imageAspectRatio = null), post,
        )).single().imageAspectRatio)
    }

    @Test
    fun linkedPostsUseAttachmentPresentationOnce() {
        val post = TimelineMedia("https://zoo.example/api/home/images/123", "photo", TimelineMediaKind.Image)
        val attachment = TimelineMedia("123", "photo", TimelineMediaKind.Image, attachmentId = "123")
        assertEquals(listOf(attachment), selectTimelineMedia(listOf(post), listOf(attachment), preferAttachments = true))
    }

    @Test
    fun sameImageAtTwoRemoteUrlsUsesOneTileAndKeepsTheSensitiveFlag() {
        val first = TimelineMedia("original", "photo", TimelineMediaKind.Image,
            url = "https://remote.example/raw.jpg", previewUrl = "$base/api/activitypub/media-images/$imageId")
        val same = first.copy(key = "alternate", url = "https://remote.example/alternate.jpg", sensitive = true)
        val shown = distinctTimelineMedia(listOf(first, same)).single()
        assertEquals(first.url, shown.url)
        assertTrue(shown.sensitive)
    }
    @Test
    fun bridgeMergesPreviouslyDistinctGroupsAndKeepsSourceUrls() {
        val first = TimelineMedia("first", "photo", TimelineMediaKind.Image, url = "$base/one")
        val second = first.copy(key = "second", url = "$base/two", sensitive = true)
        val bridge = first.copy(url = second.url, previewUrl = "$base/proxy")
        val shown = distinctTimelineMedia(listOf(first, second, bridge, second)).single()
        assertEquals(first.key, shown.key)
        assertTrue(shown.sensitive)
        assertEquals(setOf("$base/one", "$base/two", "$base/proxy"), timelineImageIdentities(listOf(first, second, bridge)))
    }

}
