package zoo.android

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue
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
