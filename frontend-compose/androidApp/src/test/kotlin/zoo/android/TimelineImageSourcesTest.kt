package zoo.android

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue
import zoo.android.ui.TimelineMedia
import zoo.android.ui.TimelineMediaKind
import zoo.obj

class TimelineImageSourcesTest {
    private val base = "https://zoo.example"
    private val imageId = "a".repeat(64)

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
}
