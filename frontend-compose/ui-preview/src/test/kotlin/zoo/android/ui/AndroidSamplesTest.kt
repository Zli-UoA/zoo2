package zoo.android.ui

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import android.view.View
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.selection.selectable
import androidx.compose.material3.RadioButton
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.sp
import java.io.File
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import org.robolectric.shadows.ShadowDialog
import zoo.GlobalSearchType

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], qualifiers = "ja-rJP-w360dp-h760dp-xhdpi")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class AndroidSamplesTest {
    @get:Rule val ui = createAndroidComposeRule<ComponentActivity>()

    @Test
    fun sensitiveVideoRevealsThePlayActionWithoutStartingPlayback() {
        var opens = 0
        ui.setContent {
            TimelineMediaTile(TimelineMedia("video", "動画", TimelineMediaKind.Video, sensitive = true), AndroidColors(false)) {
                opens++
            }
        }
        ui.onNodeWithContentDescription("再生").assertDoesNotExist()
        ui.onNodeWithContentDescription("動画を表示").performClick()
        ui.runOnIdle { assertEquals(0, opens) }
        ui.onNodeWithContentDescription("再生").performClick()
        ui.runOnIdle { assertEquals(1, opens) }
    }

    @Test
    fun sensitiveImageCanBeRevealedAndHiddenInPlace() {
        val media = TimelineMedia("photo", "写真", TimelineMediaKind.Image, sensitive = true,
            imageAspectRatio = 2f, blurhash = "LEHV6nWB2yk8pyo0adR*.7kCMdnj")
        val bitmap = Bitmap.createBitmap(8, 8, Bitmap.Config.ARGB_8888).asImageBitmap()
        var loads = 0
        ui.setContent {
            CompositionLocalProvider(LocalTimelineImage provides { loads++; bitmap }) {
                Column(Modifier.width(240.dp)) {
                    TimelineMediaTile(media, AndroidColors(false)) { error("Eye button must not open the image viewer") }
                    Text("後続の投稿")
                }
            }
        }
        val following = ui.onNodeWithText("後続の投稿")
        val before = following.getUnclippedBoundsInRoot()
        ui.onNodeWithContentDescription("写真").assertDoesNotExist()
        ui.runOnIdle { assertEquals(1, loads) }
        ui.onNodeWithContentDescription("画像を表示").performClick()
        ui.onNodeWithContentDescription("写真").assertExists()
        val hide = ui.onNodeWithContentDescription("画像を隠す")
        val imageBounds = ui.onNodeWithContentDescription("写真").getUnclippedBoundsInRoot()
        val hideBounds = hide.getUnclippedBoundsInRoot()
        org.junit.Assert.assertTrue(hideBounds.left > imageBounds.left + 120.dp)
        org.junit.Assert.assertTrue(hideBounds.top < imageBounds.top + 12.dp)
        hide.performClick()
        ui.onNodeWithContentDescription("写真").assertDoesNotExist()
        ui.onNodeWithContentDescription("画像を表示").assertExists()
        assertEquals(before, following.getUnclippedBoundsInRoot())
        ui.runOnIdle { assertEquals(1, loads) }
    }

    @Test
    fun imagePreviewReservesItsStoredAspectRatioBeforeDecoding() {
        val media = TimelineMedia("photo", "写真", TimelineMediaKind.Image, imageAspectRatio = 2f,
            blurhash = "LEHV6nWB2yk8pyo0adR*.7kCMdnj")
        // The decoder can return a different size from the stored source metadata.
        val bitmap = Bitmap.createBitmap(8, 8, Bitmap.Config.ARGB_8888).asImageBitmap()
        val decoded = CompletableDeferred<ImageBitmap>()
        ui.setContent {
            CompositionLocalProvider(LocalTimelineImage provides { decoded.await() }) {
                Column(Modifier.width(240.dp)) {
                    TimelineMediaTile(media, AndroidColors(false)) {}
                    Text("後続の投稿")
                }
            }
        }
        val blurhash = ui.onNodeWithTag("media-blurhash", useUnmergedTree = true)
        ui.waitUntil(timeoutMillis = 5_000) {
            ui.onAllNodesWithTag("media-blurhash", useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }
        blurhash.assertHeightIsEqualTo(120.dp)
        val following = ui.onNodeWithText("後続の投稿")
        val before = following.getUnclippedBoundsInRoot()
        decoded.complete(bitmap)
        ui.waitUntil(timeoutMillis = 5_000) {
            ui.onAllNodesWithTag("media-blurhash", useUnmergedTree = true).fetchSemanticsNodes().isEmpty()
        }
        ui.onNodeWithContentDescription("写真").assertHeightIsEqualTo(120.dp)
        assertEquals(before, following.getUnclippedBoundsInRoot())
    }

    @Test
    fun cachedAvatarIsComposedWithoutRunningTheLoader() {
        val bitmap = Bitmap.createBitmap(8, 8, Bitmap.Config.ARGB_8888).asImageBitmap()
        ui.setContent {
            CompositionLocalProvider(
                LocalCachedTimelineImage provides { if (it.url == "https://example.test/avatar") bitmap else null },
                LocalTimelineImage provides { error("Decoded avatar should be displayed from memory") },
            ) {
                TimelineAvatar("はる", "https://example.test/avatar", AndroidColors(false))
            }
        }
        ui.onNodeWithTag("timeline-avatar").assertExists()
    }

    @Test
    fun capturesScreensAndSendsNotificationReadOnlyInForeground() {
        val posts = listOf(
            TimelineItem("1", "はる", "Android のライト版を試しました。投稿の背景が読みやすくなりました。", "12:05"),
            TimelineItem("2", "みずき", "共有用の画像を追加します。確認をお願いします。", "11:56"),
            TimelineItem("3", "ゆい", "今日の通話テストは夕方からです。", "11:42"),
        )
        val homePosts = listOf(
            posts[0].copy(canReact = true, canRepost = true, postHeader = PostHeaderData("@haru", "11/21 12:05", PostAudience.Public, null, "misskey.io",
                remoteServer = true, serverRgb = 0x86b300)),
            posts[1].copy(canReact = true, postHeader = PostHeaderData("@mizuki", "11/21 11:56", PostAudience.Local, "開発", "zli.chat")),
            posts[2].copy(canReact = true, canRepost = true, postHeader = PostHeaderData("@yui", "11/21 11:42", PostAudience.Unlisted, null, "zli.chat")),
        )
        val settings = listOf(
            SettingsGroup("外観", listOf(SettingsEntry.Destination("theme", "テーマ", "端末に従う"),
                SettingsEntry.Destination("language", "言語", "日本語"))),
            SettingsGroup("操作", listOf(SettingsEntry.Toggle("swipe", "スワイプで返信", true),
                SettingsEntry.Toggle("haptics", "触覚フィードバック", true))),
            SettingsGroup("Zoo", listOf(SettingsEntry.Destination("general", "一般"),
                SettingsEntry.Destination("notifications", "通知"),
                SettingsEntry.Destination("navigation-cache", "画面遷移キャッシュ", "8 件"),
                SettingsEntry.Destination("update", "アプリの更新"))),
            SettingsGroup("通話", listOf(SettingsEntry.Toggle("dm-calls", "DMの着信を受ける", true),
                SettingsEntry.Destination("incoming-screen", "着信時の全画面表示を設定"))),
        )
        val channels = AndroidScreen.Channels(listOf(
            ChannelSection("参加中", listOf(ChannelItem("dev", "開発", true), ChannelItem("chat", "雑談"),
                ChannelItem("ops", "運営"), ChannelItem("news", "お知らせ"))),
            ChannelSection("ダイレクトメッセージ", listOf(ChannelItem("dm", "はる", isUserName = true))),
        ))
        val inbox = AndroidScreen.Notifications(listOf(
            InboxItem("reply", listOf(InboxActor("haru", "はる", "fixture-avatar")),
                "投稿に返信しました", "次のテストでも確認します。", "12:03", true, kind = "reply", destination = "開発"),
            InboxItem("mention", listOf(InboxActor("yui", "ゆい")),
                "開発であなたをメンションしました", "ライト版も読みやすいね。", "11:48", true, kind = "mention", destination = "開発"),
            InboxItem("reaction", listOf(InboxActor("mizuki", "みずき", "fixture-avatar"), InboxActor("nagi", "なぎ")),
                "リアクション · 2件", null, "11:22", false, kind = "like", destination = "misskey.io",
                reactions = listOf(InboxReactionEmoji("🎉"), InboxReactionEmoji(":star:", "fixture-emoji"))),
            InboxItem("follow", listOf(InboxActor("rin", "りん")), "フォローを申請しました", null, "11:15", false,
                followActor = "https://example.test/rin", followActivity = "https://example.test/follow", kind = "follow_request"),
        ))
        val conversation = AndroidScreen.Conversation("dev", "開発", "Zoo の開発と通話のテストについて相談するチャンネルです。",
            posts.mapIndexed { index, post ->
                when (index) {
                    0 -> post.copy(dateDivider = "昨日")
                    1 -> post.copy(dateDivider = "今日", unreadDivider = "新着メッセージ")
                    else -> post
                }
            }, InputState("確認しました。"))
        val avatar = Bitmap.createBitmap(128, 128, Bitmap.Config.ARGB_8888).apply {
            eraseColor(0xffd6e9df.toInt())
            val canvas = Canvas(this)
            val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply { color = 0xff38594b.toInt() }
            canvas.drawCircle(64f, 48f, 24f, paint)
            canvas.drawOval(22f, 77f, 106f, 160f, paint)
        }.asImageBitmap()
        val reactionEmoji = Bitmap.createBitmap(48, 48, Bitmap.Config.ARGB_8888).apply {
            val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
                color = 0xffedab00.toInt()
                textSize = 44f
                textAlign = Paint.Align.CENTER
            }
            Canvas(this).drawText("★", 24f, 38f, paint)
        }.asImageBitmap()
        val illustratedPost = homePosts.first().copy(
            repostedBy = "みずき", reposterAvatar = "fixture-avatar",
            postAuthor = zoo.obj("id" to "haru"),
            quote = TimelineQuote("https://example.test/notes/42", null, "なぎ", "チケットは当日も受付で購入できます。", emptyList(), handle = "@nagi@example.test",
                postAuthor = zoo.obj("id" to "nagi")),
            media = listOf(TimelineMedia("post-image", "投稿画像", TimelineMediaKind.Image, imageAspectRatio = 16f / 9f),
                TimelineMedia("post-video", "投稿動画", TimelineMediaKind.Video, sensitive = true)),
        )
        val illustratedMessage = posts.first().copy(media = listOf(
            TimelineMedia("channel-image", "添付画像", TimelineMediaKind.Image, attachmentId = "channel-image"),
            TimelineMedia("channel-video", "添付動画", TimelineMediaKind.Video, attachmentId = "channel-video"),
        ))
        val richPost = homePosts.first().copy(
            avatar = "fixture-avatar",
            reactions = listOf(TimelineReaction(":star:", ":star:", "2", false, "fixture-emoji")),
            body = "# リリースノート\n**今日の変更** は _ここ_ で確認できます。 :party:\n> 次回の予定\n~~旧手順~~ → [新しい手順](https://example.test/guide)",
            emoji = mapOf(":party:" to TimelineMedia("fixture-emoji", "party", TimelineMediaKind.Image)),
            linkCard = EntryCard("Zoo Android ガイド", "通話、通知、投稿の使い方", "https://example.test/guide", "example.test"),
        )
        val richMessage = posts.first().copy(
            body = "連携カードの代替本文",
            richContent = EntryRichContent(
                blocks = listOf(
                    EntryContentText("**ビルド結果**"),
                    EntryCard("Android の CI", "**Debug ビルド**と lint が完了しました。",
                        "https://example.test/build", "GitHub Actions",
                        listOf(EntryCardField("ブランチ", "android-ui", code = true))),
                ),
                hidesBody = true,
            ),
        )
        val searchResults = AndroidScreen.Search(SearchScreenState(
            "会議 資料", "会議 資料", GlobalSearchType.All,
            listOf(
                SearchResultItem("message:m1", "はる", "…次の**会議**の資料を共有します。`資料.md` を確認してください。", "11/21 12:05", "開発"),
                SearchResultItem("file:f1", "みずき", "会議資料_2026-11.pdf", "11/20 18:30", "開発", file = true, fileSize = "1.2 MB"),
                SearchResultItem("message:m2", "ゆい", "資料の 3 ページ目に通話テストの結果を足しました。", "11/18 09:12", "運営", inThread = true),
                SearchResultItem("message:m3", "なぎ", "資料ありがとう。明日読みます。", "2025/12/03 21:40", "はる", dm = true),
            ),
            loaded = true, canLoadOlder = true, searchedUntil = "2025-12",
            filters = searchFilterChoices,
        ))
        val channelSettings = AndroidScreen.ChannelSettings("開発", listOf(
            SettingsGroup("チャンネル", listOf(
                SettingsEntry.Information("description", "Zoo の開発と通話のテストについて相談するチャンネルです。"),
                SettingsEntry.Destination("members", "メンバー"),
                SettingsEntry.Destination("add-member", "メンバーを追加"),
                SettingsEntry.Destination("rename", "名前と説明を編集"),
                SettingsEntry.Destination("archive", "アーカイブ"),
            )),
            SettingsGroup("通知", listOf(
                SettingsEntry.Choice("notification:all", "すべての投稿", false, "通常のチャンネル投稿も届きます"),
                SettingsEntry.Choice("notification:mentions", "メンション", true, "自分への返信やリアクションも届きます"),
                SettingsEntry.Choice("notification:mute", "メンション以外を通知しない", false, "自分への明示的なメンションだけ届きます"),
            )),
        ))
        val gallery = galleryMap()
        val cases = listOf(
            "quote-renote" to AndroidScreen.NewPost(InputState("当日が楽しみです。"), emptyList(), false, "unlisted",
                quoting = illustratedPost),
            "sending-home" to AndroidScreen.Home(listOf(homePosts.first().copy(
                id = "pending", entryKey = "pending", body = "次の展示も楽しみです。", delivery = EntryDelivery.Sending,
                canReact = false, canRepost = false,
            )) + homePosts),
            "mention-channel" to conversation.copy(entries = listOf(
                posts[0].copy(body = "@mizuki 明日の受付をお願いします。", mentionsCurrentUser = true),
                posts[1].copy(body = "@here 会場は10時に開きます。"),
                posts[2].copy(body = "連絡先は desk@example.test です。"),
            ), input = InputState()),
            "typing-channel" to conversation.copy(input = InputState(notices = listOf("はるさんが入力中…"))),
            "home-rich" to AndroidScreen.Home(listOf(richPost)),
            "edit-original" to AndroidScreen.EditEntry(InputState("受付は10時半です。", expanded = true), "受付は10時です。"),
            "composer-limit" to AndroidScreen.NewPost(InputState("展示のご案内。".repeat(159)), emptyList(), false, "public"),
            "reaction-offline" to AndroidScreen.Home(listOf(illustratedPost.copy(media = emptyList(), quote = null,
                operationStatus = mapOf(EntryOperation.React to EntryOperationStatus.Offline)))),
            "repost-pending" to AndroidScreen.Home(listOf(illustratedPost.copy(media = emptyList(), quote = null,
                operationStatus = mapOf(EntryOperation.Repost to EntryOperationStatus.Sending)))),
            "embedded-post" to AndroidScreen.Home(listOf(illustratedPost.copy(media = emptyList(), body = "会場からのお知らせです。"))),
            "shared-post-link" to conversation.copy(entries = listOf(posts.first().copy(body = "昨日のお知らせです。", postLinks = listOf("https://zli.chat/home/posts/42" to "42")))),
            "article-preview" to conversation.copy(entries = listOf(posts.first().copy(body = "当日の行き方はこちらです。",
                link = "https://gallery.example/access", linkCard = EntryCard("展示会場へのアクセスと開場時間",
                    "駅から会場までの道順と、当日の入場方法をご案内します。", "https://gallery.example/access", "美術館だより",
                    image = TimelineMedia("photo", "展示会場", TimelineMediaKind.Image, url = "https://gallery.example/cover"), summaly = true)))),

            "channel-rich" to conversation.copy(entries = listOf(richMessage), input = InputState()),
            "home" to AndroidScreen.Home(homePosts),
            "home-media" to AndroidScreen.Home(listOf(illustratedPost)),
            "channel-media" to conversation.copy(entries = listOf(illustratedMessage), input = InputState()),
            "channels" to channels,
            "notifications" to inbox,
            "follow-response" to inbox.copy(items = listOf(inbox.items.last().copy(followResponse = "承認済み"))),
            "notifications-history-error" to inbox.copy(appendFailed = true),
            "channel-save-error" to AndroidScreen.EditChannel("チャンネルを編集", "開発", "展示の準備", "保存", true,
                saveError = "保存できませんでした。接続を確認して、もう一度保存してください。"),
            "post-attachment-retry" to AndroidScreen.NewPost(InputState("展示の写真"), emptyList(), false,
                attached = listOf(RecentPhoto("poster", "展示.png", true))),
            "chat-attachment-retry" to conversation.copy(input = InputState("写真を共有します",
                files = listOf("poster" to "展示.png"), thumbnails = mapOf("poster" to gallery), failedFiles = setOf("poster"))),
            "profile-follow-pending" to AndroidScreen.Profile("はる", "", emptyList(), "フォロー", followBusy = true),

            "bubble" to AndroidScreen.Bubble(posts.take(2), InputState(live = true), listOf("はる", "ゆい")),
            "settings" to AndroidScreen.Settings(settings),
            "navigation-cache" to AndroidScreen.SettingsDetail("画面遷移キャッシュ", listOf(
                SettingsGroup("", listOf(
                    SettingsEntry.Information("navigation-cache-info", "Home・投稿詳細・プロフィールと、チャンネルにそれぞれ適用するメモリ上の保持件数です。"),
                    SettingsEntry.Destination("clear-navigation-cache", "画面キャッシュを消去", "ホームと過去に開いた画面の保存内容"),
                    SettingsEntry.Choice("navigation-cache:0", "オフ", false),
                    SettingsEntry.Choice("navigation-cache:4", "4 件", false),
                    SettingsEntry.Choice("navigation-cache:8", "8 件", true),
                    SettingsEntry.Choice("navigation-cache:16", "16 件", false),
                )),
            )),
            "post-loading-replies" to AndroidScreen.Post(homePosts.first(), emptyList(), InputState(), loading = true),
            "conversation-idle" to conversation.copy(input = InputState()),
            "conversation" to conversation,
            "composer" to conversation.copy(input = InputState("確認しました。", expanded = true)),
            "thread" to AndroidScreen.Thread("開発", posts.first(), posts.drop(1), InputState()),
            "thread-settings" to AndroidScreen.SettingsDetail("スレッド設定", listOf(
                SettingsGroup("スレッド", listOf(SettingsEntry.Destination("thread-leave", "退出"),
                    SettingsEntry.Destination("thread-members", "参加者", "2"))),
            )),
            "thread-members" to AndroidScreen.Directory("スレッドの参加者", "", listOf(
                SelectionItem("haru", "はる", isUserName = true),
                SelectionItem("yui", "ゆい", isUserName = true),
            )),
            "group-dm" to AndroidScreen.Directory("ダイレクトメッセージを作成", "", listOf(
                SelectionItem("haru", "はる", isUserName = true, selected = true),
                SelectionItem("yui", "ゆい", isUserName = true, selected = true),
                SelectionItem("mizuki", "みずき", isUserName = true, selected = false),
            ), "開始", canSubmit = true, encryption = false),
            "entry-edit" to AndroidScreen.EditEntry(InputState("次のテストでも確認します。", expanded = true)),
            "people-search" to AndroidScreen.Directory("ユーザーを検索", "はる", listOf(
                SelectionItem("haru", "はる", "@haru", isUserName = true),
            )),
            "bubble-participants" to AndroidScreen.Directory("参加者", "", listOf(
                SelectionItem("haru", "はる", isUserName = true),
                SelectionItem("yui", "ゆい", isUserName = true),
            )),
            "post" to AndroidScreen.Post(homePosts.first(), homePosts.drop(1), InputState()),
            "post-compose" to AndroidScreen.NewPost(
                InputState("新しい画面を共有します。"), emptyList(), true, visibility = "public", federate = true,
            ),
            "post-compose-federation-off" to AndroidScreen.NewPost(
                InputState("新しい画面を共有します。"), emptyList(), true, visibility = "unlisted", federate = false,
            ),
            "profile" to AndroidScreen.Profile("はる", "Android 版をテストしています。写真と通話の感想を投稿します。",
                homePosts.map {
                    it.copy(author = "はる", postHeader = it.postHeader?.copy(handle = "@haru", channel = null, server = "social.example"))
                }, "フォロー", handle = "@haru@social.example", avatar = avatar, originalAvailable = true),
            "directory" to AndroidScreen.Directory("チャンネルを探す", "", listOf(SelectionItem("dev", "開発"))),
            "search-prompt" to AndroidScreen.Search(SearchScreenState("", "", GlobalSearchType.All)),
            "search" to searchResults,
            "search-files" to AndroidScreen.Search(SearchScreenState("資料", "資料", GlobalSearchType.Files,
                searchResults.state.results.filter { it.file }, loaded = true, searchedUntil = "2025-12")),
            "search-older" to AndroidScreen.Search(SearchScreenState("議事録", "議事録", GlobalSearchType.Messages,
                loaded = true, canLoadOlder = true, searchedUntil = "2025-12")),
            "search-filtered" to AndroidScreen.Search(searchResults.state.copy(filters = searchFilterChoices.copy(
                from = listOf("haru", "yui"), inChannels = listOf("dev"), mine = true))),
            "search-error" to AndroidScreen.Search(SearchScreenState("議事録", "議事録", GlobalSearchType.All, error = true)),
            "channel-edit" to AndroidScreen.EditChannel("チャンネルを作成", "開発", "", "作成", true,
                privateChannel = true, sensitive = false, encrypted = true),
            "channel-settings" to channelSettings,
            "channel-members" to AndroidScreen.Directory("メンバー", "", listOf(
                SelectionItem("haru", "はる", isUserName = true),
                SelectionItem("yui", "ゆい", isUserName = true),
            )),
            "channel-add-member" to AndroidScreen.Directory("メンバーを追加", "", listOf(
                SelectionItem("mizuki", "みずき", "@mizuki", isUserName = true),
                SelectionItem("yui", "ゆい", "@yui", isUserName = true, unavailable = "参加済み"),
            )),
            "settings-detail" to AndroidScreen.SettingsDetail("通知", settings.take(1)),
            "login" to AndroidScreen.SignIn(false),
            "login-restoring" to AndroidScreen.SignIn(true),
            "call" to AndroidScreen.Call("はる", true, avatar = avatar, hasUser = true),
            "call-active" to AndroidScreen.Call("はる", false, active = true, avatar = avatar, hasUser = true,
                minimize = true, pictureInPicture = true),
            "call-outgoing" to AndroidScreen.Call("はる", false, avatar = avatar, hasUser = true,
                minimize = true, status = "呼出中"),
        )
        var screen by mutableStateOf<AndroidScreen>(cases.first().second)
        var dark by mutableStateOf(false)
        var foreground by mutableStateOf(false)
        var emojiPicker by mutableStateOf(false)
        val actions = mutableListOf<AndroidUiAction>()
        val sharedPost = object : PostPreviewSource {
            override fun watch(id: String) {}
            override fun release(id: String) {}
            override fun retry(id: String) {}
            override fun read(id: String) = PostPreview.Loaded("なぎ", "会場は北口から徒歩5分です。受付は10時に開きます。", "fixture-avatar",
                listOf(TimelineMedia("photo", "会場案内図", TimelineMediaKind.Image, imageAspectRatio = 16f / 9f)))
        }
        ui.setContent {
            CompositionLocalProvider(LocalMentionProfiles provides mapOf("mizuki" to zoo.obj("display_name" to "みずき", "avatar_url" to "fixture-avatar")),
                LocalPostPreviews provides sharedPost, LocalTimelineImage provides { when (it.key) { "fixture-emoji" -> reactionEmoji; "photo" -> gallery; else -> avatar } }) {
                AndroidScreens(screen, dark, foreground || screen is AndroidScreen.Call, dispatch = { action ->
                    actions.add(action)
                    if (action == AndroidUiAction.UseInputTool(InputTool.Emoji)) emojiPicker = true
                    if (action == AndroidUiAction.OpenChannelSettings("dev")) {
                        screen = channelSettings
                    }
                }, language = "ja", remoteAudioLevel = { .4f }, loadNotificationAvatar = { if (it == "fixture-emoji") reactionEmoji else avatar }) { item, modifier ->
                    if (item.id == "poster") androidx.compose.foundation.Image(gallery, item.label, modifier)
                    else Box(modifier.background(Color.Gray), contentAlignment = Alignment.Center) { Text(item.label) }
                }
            }
            if (emojiPicker) {
                val choices = remember {
                    listOf(
                        "スマイリーと感情" to "😀 😊 😂 🥰 😎 🥳",
                        "アクティビティ" to "🎉 🎁 ⚽ 🎮 🎈 🎊",
                        "動物と自然" to "🐶 🦊 🐼 🐻 🐸 🐵 🦁 🐯 🐧 🦄 🦋 🐙 🌸 🌻",
                        "飲み物・食べ物" to "🍕 🍩 ☕ 🎂",
                    ).flatMap { (category, glyphs) ->
                        glyphs.split(' ').map { EmojiChoice(it, it, it, category = category) }
                    } + EmojiChoice("🐱", "🐱", "cat", keywords = "ねこ", category = "動物と自然")
                }
                EmojiPicker("絵文字", AndroidColors(dark), choices,
                    custom = listOf(EmojiChoice(":party:", "", "party", "fixture-emoji", category = "イベント")),
                    loadKitchen = {
                        KitchenChoices(listOf(EmojiChoice("1f431", "🐱", "cat"))) {
                            listOf(EmojiChoice("kitchen:1f431_1f431", "🐱🐱", "cat duo", "fixture-emoji"))
                        }
                    }, loadImage = { reactionEmoji }, dismiss = { emojiPicker = false }) { emojiPicker = false }
            }
        }
        for ((name, example) in cases) {
            for (night in listOf(false, true)) {
                ui.runOnIdle { screen = example; dark = night }
                ui.waitForIdle()
                if (name == "quote-renote") {
                    val quote = ui.onNodeWithTag("composer-quote").getUnclippedBoundsInRoot()
                    val toolbar = ui.onNodeWithTag("post-actions").getUnclippedBoundsInRoot()
                    assertEquals(12.dp, toolbar.top - quote.bottom)
                }
                if (name == "post-attachment-retry" || name == "chat-attachment-retry") {
                    fun target(label: String) = ui.onNodeWithContentDescription("展示.png" + label)
                        .fetchSemanticsNode().touchBoundsInRoot
                    check(!target("のアップロードを再試行").overlaps(target("を添付から外す")))
                    if (name == "post-attachment-retry") {
                        check(!target("の説明を編集").overlaps(target("をセンシティブに指定")))
                    }
                }
                if (name in setOf("conversation-idle", "thread")) {
                    ui.onNodeWithContentDescription("入力欄を広げる").assertDoesNotExist()
                }
                if (name == "conversation") ui.onNodeWithContentDescription("入力欄を広げる").assertIsDisplayed()
                if (name == "conversation" || name == "composer") {
                    val send = ui.onNodeWithContentDescription("送信")
                    send.assertWidthIsEqualTo(48.dp).assertHeightIsEqualTo(48.dp)
                    val bounds = send.getUnclippedBoundsInRoot()
                    val root = ui.onRoot().getUnclippedBoundsInRoot()
                    check(root.right - bounds.right == 4.dp && root.bottom - bounds.bottom == 4.dp) {
                        "Expected 4dp between send action and screen edges: $bounds / $root"
                    }
                }
                if (name == "channel-rich") {
                    ui.onNodeWithText("Android の CI", useUnmergedTree = true).assertIsDisplayed()
                    ui.onNodeWithText("連携カードの代替本文").assertDoesNotExist()
                }
                if (name == "home-rich") {
                    ui.onNodeWithText("Zoo Android ガイド", useUnmergedTree = true).assertIsDisplayed()
                    ui.onNodeWithContentDescription(":party:", useUnmergedTree = true).assertIsDisplayed()
                    ui.onNodeWithContentDescription(":star:", useUnmergedTree = true).assertIsDisplayed()
                    ui.onNodeWithTag("timeline-avatar", useUnmergedTree = true).assertIsDisplayed()
                    ui.onNodeWithContentDescription("はる", useUnmergedTree = true).assertDoesNotExist()
                }
                if (name == "channels") {
                    // The list has no search field; finding and creating channels start from the header's ＋.
                    ui.onNodeWithText("チャンネルを探す").assertDoesNotExist()
                    for (label in listOf("チャンネルを探す・作成", "チャンネルを作成", "DMを作成")) {
                        ui.onNodeWithContentDescription(label).assertHasClickAction()
                            .assertWidthIsEqualTo(48.dp).assertHeightIsEqualTo(48.dp)
                    }
                }
                if (name == "notifications") {
                    ui.onNodeWithText("未読").assertDoesNotExist()
                    ui.onNodeWithText("リアクション · 2件").assertIsDisplayed()
                    ui.onNodeWithText("🎉").assertIsDisplayed()
                    ui.onNodeWithContentDescription(":star:").assertDoesNotExist()
                    ui.onNodeWithContentDescription("みずき", useUnmergedTree = true).assertIsDisplayed()
                    ui.onNodeWithText("みずき ほか1人").assertIsDisplayed()
                    ui.onAllNodesWithContentDescription("未読", useUnmergedTree = true).assertCountEquals(2)
                }
                if (name == "home-media") {
                    val repost = ui.onNodeWithText("みずきさんがリノート", useUnmergedTree = true)
                        .assertIsDisplayed().getUnclippedBoundsInRoot()
                    val author = ui.onNodeWithText("@haru", useUnmergedTree = true).getUnclippedBoundsInRoot()
                    check(repost.left < author.left && repost.bottom <= author.top)
                    ui.onNodeWithText("なぎ").assertIsDisplayed().performClick()
                    ui.onNodeWithContentDescription("投稿画像").assertIsDisplayed()
                }
                if (name == "channel-media") {
                    ui.onNodeWithContentDescription("添付画像").assertIsDisplayed()
                    ui.onNodeWithText("動画を再生").assertIsDisplayed().performClick()
                    ui.runOnIdle {
                        check(actions.last() == AndroidUiAction.ActOnEntry(illustratedMessage, EntryOperation.Media, "channel-video"))
                    }
                }
                if (name == "home") {
                    val nameBounds = ui.onNodeWithText("@haru", useUnmergedTree = true).getUnclippedBoundsInRoot()
                    val dateBounds = ui.onNodeWithText("11/21 12:05", useUnmergedTree = true).getUnclippedBoundsInRoot()
                    val serverBounds = ui.onNodeWithText("misskey.io", useUnmergedTree = true).getUnclippedBoundsInRoot()
                    check(nameBounds.right < dateBounds.left && serverBounds.top > dateBounds.top)
                    ui.onNodeWithContentDescription("パブリック", useUnmergedTree = true).assertIsDisplayed()
                    ui.onNodeWithContentDescription("ローカルのみ", useUnmergedTree = true).assertIsDisplayed()
                    val channelBounds = ui.onNodeWithText("#開発", useUnmergedTree = true)
                        .assertIsDisplayed().getUnclippedBoundsInRoot()
                    check(channelBounds.bottom - channelBounds.top == serverBounds.bottom - serverBounds.top)
                    val channelHandle = ui.onNodeWithText("@mizuki", useUnmergedTree = true).getUnclippedBoundsInRoot()
                    check(channelBounds.top - channelHandle.bottom == serverBounds.top - nameBounds.bottom)
                    val firstPost = hasAnyAncestor(hasTestTag("timeline-entry-1"))
                    val controls = listOf("返信", "リノート", "リアクションを追加", "その他の投稿操作").map { label ->
                        ui.onNode(firstPost and hasContentDescription(label), useUnmergedTree = true)
                            .assertHasClickAction().assertIsEnabled().assertHeightIsEqualTo(36.dp).getUnclippedBoundsInRoot()
                    }
                    val postBounds = ui.onNodeWithTag("timeline-entry-1", useUnmergedTree = true).getUnclippedBoundsInRoot()
                    val bodyBounds = ui.onNode(firstPost and hasText(homePosts.first().body), useUnmergedTree = true)
                        .getUnclippedBoundsInRoot()
                    check(controls.first().left == bodyBounds.left && controls.last().right == postBounds.right - 16.dp)
                    val gaps = controls.zipWithNext { left, right -> (right.left - left.right).value }
                    check(gaps.max() - gaps.min() <= 1f)
                    ui.onNode(hasAnyAncestor(hasTestTag("timeline-entry-2")) and hasContentDescription("リノート"),
                        useUnmergedTree = true).assertIsNotEnabled()
                }
                if (name == "channels") {
                    // Full-text search is a tab of its own; the channel list keeps only its channel finder.
                    ui.onAllNodesWithContentDescription("検索").assertCountEquals(0)
                    ui.onNode(isSelectable() and hasText("検索")).assertIsNotSelected().assertHasClickAction()
                }
                if (name == "search") {
                    ui.onNode(isSelectable() and hasText("検索")).assertIsSelected()
                    ui.onNodeWithText("暗号化されたチャンネルは検索の対象外です。").assertIsDisplayed()
                    // Terms are marked in the snippet's text and in file names, but not inside code.
                    fun marked(prefix: String) = ui.onNode(hasText(prefix, substring = true), useUnmergedTree = true)
                        .fetchSemanticsNode().config[SemanticsProperties.Text].single().let { text ->
                            text.spanStyles.filter { it.item.background != Color.Unspecified }
                                .map { text.text.substring(it.start, it.end) }
                        }
                    check(marked("…次の会議") == listOf("会議", "資料")) { marked("…次の会議").toString() }
                    check(marked("会議資料_2026") == listOf("会議", "資料")) { marked("会議資料_2026").toString() }
                    // The kind is the first filter: "all" by default, shown but not filled.
                    ui.onNodeWithTag("search-filter-type").assertTextEquals("すべて").assertIsNotSelected()
                        .assertHeightIsEqualTo(48.dp)
                    ui.onNodeWithText("運営 · スレッドの返信", useUnmergedTree = true).assertIsDisplayed()
                    ui.onNodeWithText("1.2 MB", useUnmergedTree = true).assertIsDisplayed()
                    ui.onNodeWithTag("global-search-results").performScrollToNode(hasText("さらに古い結果を検索"))
                    ui.onNodeWithText("2025-12 まで検索しました").assertIsDisplayed()
                    ui.onNodeWithText("さらに古い結果を検索").assertHeightIsEqualTo(48.dp)
                    ui.onNodeWithTag("global-search-results").performScrollToIndex(0)
                }
                if (name == "search-files") {
                    ui.onNodeWithTag("search-filter-type").assertTextEquals("ファイル").assertIsSelected()
                    ui.onNodeWithText("2025-12 まで検索しました").assertIsDisplayed()
                    ui.onNodeWithText("さらに古い結果を検索").assertDoesNotExist()
                }
                if (name == "search-older") {
                    ui.onNodeWithText("2025-12 までに見つかりませんでした。さらに古い期間を検索できます。").assertIsDisplayed()
                    ui.onNodeWithText("さらに古い結果を検索").assertIsDisplayed()
                }
                if (name == "search-error") ui.onNodeWithText("再試行").assertHasClickAction()
                if (name == "search-filtered") {
                    // Chosen filters name what they narrow to, as Web's picker buttons.
                    ui.onNodeWithText("送信者: 2 件").assertIsDisplayed()
                    ui.onNodeWithText("チャンネル: 開発").assertIsDisplayed()
                    ui.onNode(isToggleable() and hasText("自分が参加中")).assertIsOn()
                    ui.onNode(isToggleable() and hasText("ボットを除く")).assertIsOff()
                }
                if (name == "search-prompt") {
                    ui.onNodeWithText("メッセージとファイルを検索").assertIsDisplayed()
                    ui.onAllNodesWithText("さらに古い結果を検索").assertCountEquals(0)
                }
                if (name == "profile") {
                    ui.onNodeWithText("@haru@social.example").assertIsDisplayed()
                    ui.onNodeWithContentDescription("公開プロフィールを開く").assertHasClickAction()
                    ui.onNodeWithText("フォロー").assertHasClickAction()
                }
                if (name == "post-compose") {
                    val audience = ui.onNodeWithContentDescription("公開範囲：パブリック").assertHasClickAction()
                    val federation = ui.onNodeWithContentDescription("ActivityPub で配信").assertIsOn()
                    check(audience.getUnclippedBoundsInRoot().top == federation.getUnclippedBoundsInRoot().top)
                    val toolbar = ui.onNodeWithTag("post-actions").getUnclippedBoundsInRoot()
                    val post = ui.onNodeWithText("投稿").assertHeightIsEqualTo(48.dp).getUnclippedBoundsInRoot()
                    val photos = ui.onNodeWithContentDescription("写真").getUnclippedBoundsInRoot()
                    check(
                        post.top - toolbar.top == 8.dp && toolbar.bottom - post.bottom == 8.dp &&
                            toolbar.right - post.right == 8.dp && photos.left - toolbar.left == 8.dp,
                    ) { "Expected an 8dp inset around the post actions: $post / $photos / $toolbar" }
                }
                if (name == "post-compose-federation-off") {
                    ui.onNodeWithContentDescription("公開範囲：ホーム").assertIsDisplayed()
                    ui.onNodeWithContentDescription("ActivityPub で配信").assertIsOff()
                }
                if (name == "call") {
                    ui.onNodeWithText("応答").assertHasClickAction()
                    ui.onNodeWithText("拒否").assertHasClickAction()
                    val answer = ui.onNodeWithTag("incoming-call-応答", useUnmergedTree = true)
                        .assertWidthIsEqualTo(76.dp).assertHeightIsEqualTo(76.dp).getUnclippedBoundsInRoot()
                    val decline = ui.onNodeWithTag("incoming-call-拒否", useUnmergedTree = true)
                        .assertWidthIsEqualTo(76.dp).assertHeightIsEqualTo(76.dp).getUnclippedBoundsInRoot()
                    check(answer.right < decline.left && answer.top == decline.top)
                }
                if (name == "call-active") {
                    ui.onNodeWithText("ミュート").assertIsOff()
                    ui.onNodeWithText("スピーカー").assertIsOff()
                    ui.onNodeWithContentDescription("通話画面をしまう").assertHasClickAction()
                    ui.onNodeWithContentDescription("通話を小窓で表示").assertHasClickAction()
                }
                if (example is AndroidScreen.Call) {
                    if (example.active) ui.mainClock.advanceTimeBy(150)
                    ui.onNodeWithTag("call-avatar").assertWidthIsEqualTo(88.dp).assertHeightIsEqualTo(88.dp)
                }
                capture("$name-${if (night) "dark" else "light"}")
            }
        }
        for (night in listOf(false, true)) {
            ui.runOnIdle { screen = cases.first { it.first == "post-compose" }.second; dark = night }
            ui.onNodeWithContentDescription("絵文字").performClick()
            ui.onNodeWithText("🎉").assertIsDisplayed()
            ui.onNodeWithText("スマイリーと感情").assertIsDisplayed()
            ui.onNodeWithText("アクティビティ").assertIsDisplayed()
            capture("emoji-picker-${if (night) "dark" else "light"}", bottomDialog = true)
            ui.onNodeWithTag("emoji-search").performTextInput("cat")
            ui.onNodeWithContentDescription("cat").assertIsDisplayed()
            ui.onNodeWithText("🎉").assertDoesNotExist()
            ui.onNodeWithText("カスタム").performClick()
            ui.onNodeWithContentDescription("party").assertIsDisplayed()
            ui.onNodeWithText("イベント").assertIsDisplayed()
            capture("emoji-picker-custom-${if (night) "dark" else "light"}", bottomDialog = true)
            ui.onNodeWithText("Emoji Kitchen").performClick()
            ui.onNodeWithContentDescription("cat").performClick()
            ui.onNodeWithContentDescription("cat duo").assertIsDisplayed()
            capture("emoji-picker-kitchen-${if (night) "dark" else "light"}", bottomDialog = true)
            ui.onNodeWithContentDescription("cat duo").performClick()
        }
        ui.runOnIdle { screen = cases.first { it.first == "post-compose" }.second }
        ui.onNodeWithContentDescription("公開範囲：パブリック").performClick()
        ui.onNodeWithText("パブリック").assertIsSelected()
        captureMenu("post-audience-menu")
        for (audience in listOf("ホーム", "フォロワー")) {
            ui.onNodeWithText(audience).assertIsDisplayed().assertIsNotSelected()
        }
        ui.onNodeWithText("ローカルのみ").assertDoesNotExist()
        ui.onNodeWithText("ホーム").performClick()
        ui.onNodeWithContentDescription("ActivityPub で配信").performClick()
        ui.runOnIdle {
            check(actions.takeLast(2) == listOf(
                AndroidUiAction.SelectPostVisibility("unlisted"),
                AndroidUiAction.OpenSetting("toggle-federation"),
            ))
            screen = AndroidScreen.Home(homePosts)
        }
        val firstPost = hasAnyAncestor(hasTestTag("timeline-entry-1"))
        ui.onNode(firstPost and hasContentDescription("返信"), useUnmergedTree = true).performTouchInput {
            click(center.copy(x = center.x + 15.dp.toPx()))
        }
        for (label in listOf("リノート", "リアクションを追加", "その他の投稿操作")) {
            ui.onNode(firstPost and hasContentDescription(label), useUnmergedTree = true).performClick()
        }
        ui.onNodeWithText("本文をコピー").assertIsDisplayed()
        ui.runOnIdle {
            val entry = homePosts.first()
            check(actions.takeLast(3) == listOf(
                AndroidUiAction.ReplyToPost(entry.id),
                AndroidUiAction.ActOnEntry(entry, EntryOperation.Repost),
                AndroidUiAction.ActOnEntry(entry, EntryOperation.React),
            ))
        }
        ui.runOnIdle { screen = channels }
        for (item in listOf("チャンネルを探す", "チャンネルを作成")) {
            ui.onNodeWithContentDescription("チャンネルを探す・作成").performClick()
            ui.onNodeWithText(item).performClick()
            ui.onNodeWithText(item).assertDoesNotExist()
        }
        ui.runOnIdle {
            check(actions.takeLast(2) == listOf(AndroidUiAction.Browse, AndroidUiAction.CreateChannel)) { actions.takeLast(2).toString() }
        }
        ui.onNode(isSelectable() and hasText("検索")).performClick()
        ui.runOnIdle { screen = searchResults }
        ui.onNodeWithTag("global-search-field").performTextReplacement("会議")
        ui.onNodeWithTag("search-filter-type").performClick()
        ui.onNode(hasText("すべて") and isSelected()).assertExists()
        captureMenu("search-type-menu")
        ui.onNodeWithText("メッセージ").performClick()
        ui.onNodeWithText("メッセージ").assertDoesNotExist()
        ui.onNodeWithText("みずき", useUnmergedTree = true).performClick()
        ui.onNodeWithTag("global-search-results").performScrollToNode(hasText("さらに古い結果を検索"))
        ui.onNodeWithText("さらに古い結果を検索").performClick()
        ui.runOnIdle {
            check(actions.takeLast(5) == listOf(
                AndroidUiAction.Navigate(TopPage.Search),
                AndroidUiAction.SearchMessages("会議"),
                AndroidUiAction.SelectSearchType(GlobalSearchType.Messages),
                AndroidUiAction.OpenSearchResult("file:f1"),
                AndroidUiAction.LoadOlderResults,
            )) { "Search actions: ${actions.takeLast(5)}" }
        }
        ui.runOnIdle { screen = cases.first { it.first == "group-dm" }.second }
        ui.onNodeWithText("はる").performClick()
        ui.onNodeWithText("開始").performClick()
        ui.runOnIdle {
            check(AndroidUiAction.SelectResult("haru") in actions)
            check(AndroidUiAction.OpenSetting("submit-dm") in actions)
            screen = conversation
        }
        ui.onNodeWithText("開発").assertHasClickAction().performClick()
        for (label in listOf("すべての投稿", "メンション", "メンション以外を通知しない")) {
            ui.onNodeWithText(label).assertIsDisplayed()
        }
        ui.onNodeWithText("メンション以外を通知しない").performClick()
        ui.runOnIdle {
            check(actions.contains(AndroidUiAction.OpenChannelSettings("dev")))
            check(actions.contains(AndroidUiAction.OpenSetting("notification:mute")))
            check(actions.none { it is AndroidUiAction.ViewedNotification })
            screen = inbox
            foreground = true
        }
        ui.waitForIdle()
        ui.mainClock.advanceTimeBy(650)
        ui.runOnIdle {
            val readIds = actions.filterIsInstance<AndroidUiAction.ViewedNotification>().map { it.id }
            check(readIds.sorted() == listOf("mention", "reply")) { "Read actions: $readIds" }
        }
        ui.onNodeWithText("すべて既読").assertDoesNotExist()
    }

    /** Shows the search screen with filter choices; the dispatch reflects choices as the adapter would. */
    private fun showSearchFilters(actions: MutableList<AndroidUiAction>): (Boolean) -> Unit {
        var screen by mutableStateOf(AndroidScreen.Search(SearchScreenState("会議", "会議", GlobalSearchType.All,
            loaded = true, filters = searchFilterChoices)))
        var dark by mutableStateOf(false)
        ui.setContent {
            AndroidScreens(screen, dark, true, dispatch = { action ->
                actions += action
                // Reflect the choice as the adapter would, so the sheet shows it.
                val filters = screen.state.filters
                when (action) {
                    is AndroidUiAction.SelectSearchFilter -> screen = AndroidScreen.Search(screen.state.copy(filters =
                        if (action.field == SearchFilterField.From) filters.copy(from = action.ids) else filters.copy(inChannels = action.ids)))
                    is AndroidUiAction.ToggleSearchFilter -> screen = AndroidScreen.Search(screen.state.copy(filters =
                        if (action.flag == SearchFilterFlag.Mine) filters.copy(mine = !filters.mine) else filters.copy(noBots = !filters.noBots)))
                    else -> Unit
                }
            }, language = "ja") { _, _ -> }
        }
        return { ui.runOnIdle { dark = it } }
    }

    @Test
    fun searchFiltersPickSendersInASheet() {
        val actions = mutableListOf<AndroidUiAction>()
        val setDark = showSearchFilters(actions)
        ui.onNodeWithTag("search-filter-from").assertHeightIsAtLeast(48.dp).assertIsNotSelected().performClick()
        // The user comes first, marked as themselves.
        ui.onNodeWithText("はる（自分）").assertIsDisplayed()
        ui.onNodeWithText("ゆい").performClick()
        capture("search-filter-sheet-light", withSheet = true)
        setDark(true)
        capture("search-filter-sheet-dark", withSheet = true)
        setDark(false)
        ui.onNodeWithTag("search-filter-field").performTextInput("@nagi")
        ui.onNodeWithText("はる（自分）").assertDoesNotExist()
        ui.onNodeWithText("なぎ").performClick()
        ui.onNodeWithText("選択を外す").performClick()
        ui.runOnIdle {
            check(actions.filterIsInstance<AndroidUiAction.SelectSearchFilter>().map { it.ids } ==
                listOf(listOf("yui"), listOf("yui", "nagi"), emptyList())) { actions.toString() }
        }
    }

    @Test
    fun searchFiltersSwitchFlagsAndPickChannels() {
        val actions = mutableListOf<AndroidUiAction>()
        showSearchFilters(actions)
        // The row scrolls sideways on narrow screens.
        ui.onNode(isToggleable() and hasText("ボットを除く")).performScrollTo().performClick().assertIsOn()
        ui.onNodeWithTag("search-filter-in").performScrollTo().performClick()
        ui.onNodeWithText("雑談").assertIsDisplayed()
        ui.onNodeWithTag("search-filter-in").assertIsNotSelected()
        ui.onNodeWithText("運営").performClick()
        ui.onNodeWithText("チャンネル: 運営").assertExists()
        // The chosen filter is exposed as selected, not only by its colour.
        ui.onNodeWithTag("search-filter-in").assertIsSelected()
        ui.runOnIdle {
            check(actions == listOf(
                AndroidUiAction.ToggleSearchFilter(SearchFilterFlag.NoBots),
                AndroidUiAction.SelectSearchFilter(SearchFilterField.In, listOf("ops")),
            )) { actions.toString() }
        }
    }

    @Test
    fun opensContextSheetAndSlidesToThreadWithOptionalHaptics() {
        val root = TimelineItem("root", "はる", "スレッドを試すメッセージ", "12:08",
            editable = true, canReact = true)
        val conversation = AndroidScreen.Conversation("dev", "開発", "テスト用",
            listOf(root), InputState())
        val thread = AndroidScreen.Thread("開発", root, emptyList(), InputState())
        var screen by mutableStateOf<AndroidScreen>(conversation)
        var haptics by mutableStateOf(false)
        var dark by mutableStateOf(false)
        val pulses = mutableListOf<HapticFeedbackType>()
        val feedback = object : HapticFeedback {
            override fun performHapticFeedback(hapticFeedbackType: HapticFeedbackType) {
                pulses.add(hapticFeedbackType)
            }
        }
        ui.setContent {
            CompositionLocalProvider(LocalHapticFeedback provides feedback) {
                AndroidScreens(screen, dark, true, dispatch = { action ->
                    when (action) {
                        AndroidUiAction.OpenThread(root.id) -> screen = thread
                        AndroidUiAction.Back -> screen = conversation
                        else -> Unit
                    }
                }, language = "ja", interaction = AndroidInteractionSettings(haptics = haptics)) { _, _ -> }
            }
        }
        val message = ui.onNodeWithTag("timeline-entry-root")
        message.performTouchInput { longClick() }
        ui.onNodeWithText("本文をコピー").assertIsDisplayed()
        ui.onNodeWithText("編集").assertIsDisplayed()
        ui.onNodeWithText("削除").assertIsDisplayed()
        ui.onNodeWithText("リアクション").assertIsDisplayed()
        capture("message-menu-light", withSheet = true)
        ui.runOnIdle { dark = true; check(pulses.isEmpty()) }
        ui.onNodeWithText("本文をコピー").assertIsDisplayed()
        capture("message-menu-dark", withSheet = true)
        ui.onNodeWithText("スレッドを開く").performClick()
        ui.onNodeWithText("スレッド").assertIsDisplayed()
        ui.onNodeWithContentDescription("戻る").performClick()
        ui.onNodeWithText("開発").assertIsDisplayed()
        ui.runOnIdle { haptics = true }
        ui.waitForIdle()
        message.performTouchInput { swipeLeft(startX = width * .8f, endX = width * .15f) }
        ui.onNodeWithText("スレッド").assertIsDisplayed()
        ui.runOnIdle { check(pulses.size == 1) { "Expected a single threshold pulse: $pulses" } }
        ui.onNodeWithContentDescription("戻る").performClick()
        ui.onNodeWithText("開発").assertIsDisplayed()
        ui.runOnIdle { screen = AndroidScreen.Home(listOf(root.copy(canRepost = true))); dark = false }
        message.performTouchInput { longClick() }
        ui.onNodeWithText("投稿を開く").assertIsDisplayed()
        ui.onNodeWithText("リノート").assertIsDisplayed()
        capture("post-menu-light", withSheet = true)
        ui.runOnIdle { dark = true }
        capture("post-menu-dark", withSheet = true)
    }

    @Test
    fun homeTabsAndAuthorAvatarsDispatchTheirOwnActions() {
        val author = zoo.obj("id" to "haru")
        val quotedAuthor = zoo.obj("id" to "https://social.example/users/yui")
        val post = TimelineItem("post", "はる", "投稿本文", "12:05", postAuthor = author,
            postHeader = PostHeaderData("@haru", "12:05", PostAudience.Public, null, null),
            quote = TimelineQuote("https://social.example/posts/1", null, "ゆい", "引用本文", emptyList(),
                postAuthor = quotedAuthor, handle = "@yui@social.example"))
        val actions = mutableListOf<AndroidUiAction>()
        ui.setContent {
            AndroidScreens(AndroidScreen.Home(listOf(post), timeline = "home"), false, true,
                dispatch = { actions.add(it) }, language = "ja") { _, _ -> }
        }
        val quoteName = ui.onNodeWithText("ゆい", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot
        val handle = ui.onNodeWithText("@yui@social.example", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot
        val body = ui.onNodeWithText("引用本文", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot
        check(handle.left >= quoteName.right && handle.center.y in quoteName.top..quoteName.bottom)
        val avatar = ui.onNodeWithTag("quote-avatar", useUnmergedTree = true).fetchSemanticsNode().boundsInRoot
        check(body.left == avatar.left && body.top >= quoteName.bottom)
        fun baseline(label: String): Float {
            val node = ui.onNodeWithText(label, useUnmergedTree = true)
            val layouts = mutableListOf<androidx.compose.ui.text.TextLayoutResult>()
            node.performSemanticsAction(androidx.compose.ui.semantics.SemanticsActions.GetTextLayoutResult) { it(layouts) }
            return node.fetchSemanticsNode().boundsInRoot.top + layouts.single().firstBaseline
        }
        check(kotlin.math.abs(baseline("ゆい") - baseline("@yui@social.example")) <= 1f)
        ui.onNode(hasText("ホーム") and isSelectable() and isSelected() and hasAnySibling(hasText("ローカル"))).assertIsDisplayed()
        capture("home-timeline-tabs")
        ui.onNodeWithText("ローカル").performClick()
        ui.onNodeWithText("ソーシャル").performClick()
        for (authorName in listOf("はる", "ゆい")) {
            ui.onNodeWithContentDescription("「${authorName}」のプロフィールを開く")
                .assertWidthIsAtLeast(48.dp).assertHeightIsAtLeast(if (authorName == "ゆい") 28.dp else 48.dp)
                .performTouchInput { click(center.copy(x = width - 1f)) }
        }
        for (name in listOf("はる", "@haru")) {
            ui.onNodeWithText(name, useUnmergedTree = true).performTouchInput { click() }
        }
        ui.onNodeWithText("引用本文", useUnmergedTree = true).performTouchInput { click() }
        ui.onNodeWithText("投稿本文", useUnmergedTree = true).performTouchInput { click() }
        ui.runOnIdle {
            check(actions == listOf(
                AndroidUiAction.SelectTimeline("local"), AndroidUiAction.SelectTimeline("social"),
                AndroidUiAction.OpenProfile(author), AndroidUiAction.OpenProfile(quotedAuthor),
                AndroidUiAction.OpenProfile(author), AndroidUiAction.OpenProfile(author),
                AndroidUiAction.ActOnEntry(post, EntryOperation.Quote), AndroidUiAction.OpenPost(post.id),
            )) { actions.toString() }
        }
    }

    @Test
    fun closingAnAuthorProfileRestoresTheHomeAndPostTimelinePositions() {
        val posts = (0..40).map { index ->
            TimelineItem("post-$index", "投稿者 $index", "投稿 $index", "12:05",
                postAuthor = zoo.obj("id" to "author-$index"))
        }
        val home = AndroidScreen.Home(posts, timeline = "local")
        val thread = AndroidScreen.Post(posts.first(), posts.drop(1), InputState())
        var previous: AndroidScreen = home
        var screen by mutableStateOf<AndroidScreen>(home)
        ui.setContent {
            AndroidScreens(screen, false, true, dispatch = { action ->
                when (action) {
                    is AndroidUiAction.OpenProfile -> screen = AndroidScreen.Profile("投稿者 24", "", emptyList())
                    AndroidUiAction.Back -> screen = previous
                    else -> Unit
                }
            }, language = "ja") { _, _ -> }
        }
        for (surface in listOf(home, thread)) {
            ui.runOnIdle { previous = surface; screen = surface }
            ui.onNode(hasScrollAction() and hasAnyDescendant(hasTestTag("timeline-entry-post-0")))
                .performScrollToIndex(24)
            ui.onNodeWithContentDescription("「投稿者 24」のプロフィールを開く").performClick()
            ui.onNodeWithContentDescription("戻る").performClick()
            ui.onNodeWithTag("timeline-entry-post-24").assertIsDisplayed()
            ui.onNodeWithTag("timeline-entry-post-0").assertDoesNotExist()
        }
    }

    @Test
    fun profileKeepsItsScrollPositionWhileThePostsAreRevalidated() {
        val posts = (0..40).map { index -> TimelineItem("post-$index", "はる", "投稿 $index", "12:05") }
        val profile = AndroidScreen.Profile("はる", "", posts, id = "haru")
        val loading = profile.copy(posts = emptyList(), postsPending = true, online = false)
        var screen by mutableStateOf<AndroidScreen>(profile)
        ui.setContent {
            AndroidScreens(screen, false, true, dispatch = { action ->
                when (action) {
                    is AndroidUiAction.OpenPost -> screen = AndroidScreen.Post(posts[24], emptyList(), InputState())
                    AndroidUiAction.Back -> screen = loading
                    else -> Unit
                }
            }, language = "ja") { _, _ -> }
        }

        ui.onNode(hasScrollAction()).performScrollToIndex(25)
        ui.onNodeWithTag("timeline-entry-post-24").performClick()
        ui.onNodeWithContentDescription("戻る").performClick()
        ui.onNodeWithText("オフラインです").assertIsDisplayed()
        ui.onNodeWithTag("timeline-entry-post-24").assertDoesNotExist()
        ui.runOnIdle { screen = profile }
        ui.onNodeWithTag("timeline-entry-post-24").assertIsDisplayed()
        ui.onNodeWithTag("timeline-entry-post-0").assertDoesNotExist()
    }

    @Test
    fun profileRefreshFailureShowsRetryInsteadOfAnEmptyFeed() {
        val actions = mutableListOf<AndroidUiAction>()
        ui.setContent {
            AndroidScreens(AndroidScreen.Profile("はる", "", emptyList(), postsPending = true, postsLoadFailed = true),
                false, true, dispatch = { actions.add(it) }, language = "ja") { _, _ -> }
        }
        ui.onNodeWithText("表示できる投稿はありません").assertDoesNotExist()
        ui.onNodeWithText("再試行").assertIsDisplayed().performClick()
        ui.runOnIdle { check(actions == listOf(AndroidUiAction.RetryProfile)) }
    }

    @Test
    fun feedsOfferRetryWithoutRemovingLoadedPosts() {
        val actions = mutableListOf<AndroidUiAction>()
        val post = TimelineItem("post", "はる", "取得済みの本文", "12:05")
        var screen by mutableStateOf<AndroidScreen>(AndroidScreen.Home(emptyList(), timeline = "home", loadFailed = true))
        ui.setContent {
            AndroidScreens(screen, false, true, dispatch = { actions.add(it) }, language = "ja") { _, _ -> }
        }
        ui.onNodeWithText("タイムラインを取得できませんでした。").assertIsDisplayed()
        ui.onNodeWithText("再試行").assertIsDisplayed().performClick()
        ui.runOnIdle { screen = AndroidScreen.Home(listOf(post), timeline = "home", loadFailed = true) }
        ui.onNodeWithText("取得済みの本文").assertIsDisplayed()
        ui.onNodeWithText("再試行").performClick()
        ui.runOnIdle { screen = AndroidScreen.Post(post, emptyList(), InputState(), loadFailed = true) }
        ui.onNodeWithText("取得済みの本文").assertIsDisplayed()
        ui.onNodeWithText("再試行").performClick()
        ui.runOnIdle {
            check(actions == listOf(AndroidUiAction.RetryTimeline, AndroidUiAction.RetryTimeline, AndroidUiAction.RetryPost))
        }
    }

    @Test
    fun unavailableLocalQuoteHasNoExternalLink() {
        ui.setContent {
            TimelineQuoteCard(TimelineQuote("https://zoo.example/home/posts/removed", null, "", "", emptyList(), local = true),
                AndroidColors(false), open = { error("Unavailable quote was opened") }, openMedia = {})
        }
        ui.onNodeWithText("この投稿は表示できません").assertIsDisplayed().assertHasNoClickAction()
        ui.onNodeWithText("引用元を開く").assertDoesNotExist()
    }

    @Test
    fun renoteMenuOpensQuoteComposerWithOriginal() {
        val post = TimelineItem("original", "ゆい", "展示は明日からです。", "12:00", canRepost = true, canQuoteRenote = true)
        var screen by mutableStateOf<AndroidScreen>(AndroidScreen.Home(listOf(post)))
        ui.setContent {
            AndroidScreens(screen, false, true, dispatch = { action ->
                if (action is AndroidUiAction.ActOnEntry) {
                    check(action.item.id == post.id && action.operation == EntryOperation.QuoteRenote)
                    screen = AndroidScreen.NewPost(InputState("楽しみです。"), emptyList(), false, quoting = post)
                }
            }, language = "ja") { _, _ -> }
        }
        ui.onNodeWithContentDescription("リノート").performClick()
        ui.onNodeWithText("リノート").assertIsDisplayed()
        ui.onNodeWithText("引用リノート").performClick()
        ui.onNodeWithText("展示は明日からです。").assertIsDisplayed()
        ui.onNodeWithText("楽しみです。").assertIsDisplayed()
    }

    @Test
    fun pendingRepostDisablesOnlyRepostingAndFailureAllowsRetry() {
        var status by mutableStateOf(EntryOperationStatus.Sending)
        val actions = mutableListOf<AndroidUiAction>()
        ui.setContent {
            val entry = TimelineItem("post", "なぎ", "展示のお知らせ", "12:00", canReact = true, canRepost = true,
                operationStatus = mapOf(EntryOperation.Repost to status))
            AndroidScreens(AndroidScreen.Home(listOf(entry)), false, true, dispatch = actions::add, language = "ja") { _, _ -> }
        }
        ui.onNodeWithText("リノートを反映中…").assertIsDisplayed()
        ui.onNodeWithContentDescription("リノート").assertIsNotEnabled()
        ui.onNodeWithContentDescription("リアクションを追加").assertIsEnabled()
        ui.runOnIdle { status = EntryOperationStatus.Denied }
        ui.onNodeWithText("リノートの権限がありません").assertIsDisplayed()
        ui.onNodeWithContentDescription("リノート").performClick()
        ui.runOnIdle { check(actions.filterIsInstance<AndroidUiAction.ActOnEntry>().single().operation == EntryOperation.Repost) }
    }

    @Test
    fun progressDoesNotMoveTheFollowingControl() {
        var active by mutableStateOf(false)
        ui.setContent {
            androidx.compose.foundation.layout.Column {
                ProgressStrip(active, Color.Black)
                Text("次の操作")
            }
        }
        val control = ui.onNodeWithText("次の操作")
        val initial = control.fetchSemanticsNode().boundsInRoot
        for (value in listOf(true, false)) {
            ui.runOnIdle { active = value }
            check(control.fetchSemanticsNode().boundsInRoot == initial)
        }
    }

    @Test
    fun imageDescriptionStaysWithItsAttachment() {
        var description by mutableStateOf("")
        var sensitive by mutableStateOf(false)
        var dark by mutableStateOf(false)
        ui.setContent {
            AndroidScreens(AndroidScreen.NewPost(InputState(), emptyList(), false,
                attached = listOf(RecentPhoto("map", "map.png", description = description, sensitive = sensitive))),
                dark, true, dispatch = { action ->
                    if (action is AndroidUiAction.SetPhotoSensitive) sensitive = action.sensitive
                    if (action is AndroidUiAction.DescribePhoto) {
                        check(action.id == "map")
                        description = action.description
                    }
                }, language = "ja") { item, modifier ->
                androidx.compose.foundation.Image(remember { galleryMap() }, item.label, modifier)
            }
        }
        ui.onNodeWithContentDescription("map.pngをセンシティブに指定").performClick()
        ui.onNodeWithContentDescription("map.pngのセンシティブ指定を解除").assertIsDisplayed()
        ui.onNodeWithContentDescription("map.pngの説明を編集").performClick()
        ui.onAllNodes(hasSetTextAction()).onLast().performSemanticsAction(
            androidx.compose.ui.semantics.SemanticsActions.SetText
        ) { set -> check(set(androidx.compose.ui.text.AnnotatedString("北口から受付までの地図"))) }
        for (night in listOf(false, true)) {
            ui.runOnIdle { dark = night }
            ui.onNodeWithText("北口から受付までの地図").assertIsDisplayed()
            capture("image-description-" + if (night) "dark" else "light")
        }
        ui.onNodeWithText("完了").performClick()
        ui.onNodeWithContentDescription("map.pngの説明を編集").performClick()
        ui.onNodeWithText("北口から受付までの地図").assertIsDisplayed()
    }

    @Test
    fun unavailableReactionRemainsVisibleButCannotBeTapped() {
        var toggles = 0
        ui.setContent {
            TimelineReactionChip(TimelineReaction(":star:", ":star:", "2", false, canToggle = false), AndroidColors(false)) {
                toggles++
            }
        }
        ui.onNodeWithText(":star:").assertIsDisplayed().assertIsNotEnabled().performTouchInput { click() }
        ui.runOnIdle { check(toggles == 0) }
    }

    @Test
    fun emojiPickerFitsSixColumnsOnPhone() {
        assertEmojiColumns(6)
    }

    @Test
    @Config(qualifiers = "ja-rJP-w720dp-h760dp-xhdpi")
    fun emojiPickerUsesFullAvailableWidthOnTablet() {
        assertEmojiColumns(14)
        capture("emoji-picker-wide-light", bottomDialog = true)
    }

    private fun assertEmojiColumns(expected: Int) {
        val choices = ("😀 😃 😄 😁 😆 😅 😂 🙂 🙃 😉 😊 😇 🥰 😍 🤩 😘 😋 😛 😜 🤪 " +
            "😎 🤓 🧐 🥳 👍 👋 🙌 👏 ✨ 🔥 🎉 🎈 🐶 🐱 🦊 🐼 🐻 🌈 🚀 🌍 🌸 🍕")
            .split(' ').mapIndexed { i, glyph -> EmojiChoice("emoji-$i", glyph, "emoji-$i") }
        ui.setContent {
            EmojiPicker(
                "絵文字", AndroidColors(dark = false), choices, custom = emptyList(),
                loadKitchen = { null }, loadImage = { null }, dismiss = {}, select = {},
            )
        }
        val first = ui.onNodeWithContentDescription("emoji-0").getUnclippedBoundsInRoot()
        val last = ui.onNodeWithContentDescription("emoji-${expected - 1}").getUnclippedBoundsInRoot()
        val next = ui.onNodeWithContentDescription("emoji-$expected").getUnclippedBoundsInRoot()
        check(last.top == first.top && next.top > first.top)
        check(first.right - first.left >= 48.dp)
    }

    private val searchFilterChoices = SearchFilterState(
        people = listOf(
            SearchFilterOption("haru", "はる（自分）", "@haru"),
            SearchFilterOption("mizuki", "みずき", "@mizuki"),
            SearchFilterOption("nagi", "なぎ", "@nagi"),
            SearchFilterOption("yui", "ゆい", "@yui"),
        ),
        channels = listOf(
            SearchFilterOption("dev", "開発", mark = SearchFilterMark.Channel),
            SearchFilterOption("ops", "運営", mark = SearchFilterMark.PrivateChannel),
            SearchFilterOption("random", "雑談", mark = SearchFilterMark.Channel),
        ),
    )

    @Test
    fun keyboardCanOpenReactionMenuOnATextOnlyChannelMessage() {
        val post = TimelineItem("plain", "はる", "本文", "12:05", canReact = true)
        val actions = mutableListOf<AndroidUiAction>()
        ui.setContent {
            AndroidScreens(AndroidScreen.Conversation("dev", "開発", "", listOf(post), InputState("")),
                false, true, dispatch = actions::add, language = "ja") { _, _ -> }
        }
        val row = ui.onNodeWithTag("timeline-entry-plain")
        row.performSemanticsAction(SemanticsActions.RequestFocus)
        row.assertIsFocused().performKeyInput { pressKey(Key.Enter) }
        ui.onNodeWithText("リアクション").performClick()
        ui.runOnIdle {
            assertEquals(AndroidUiAction.ActOnEntry(post, EntryOperation.React, ""),
                actions.filterIsInstance<AndroidUiAction.ActOnEntry>().single())
        }
    }

    @Test
    fun reactionsCanBeSelectedRemovedAndAddedOnHomeAndChannel() {
        var channel by mutableStateOf(false)
        var dark by mutableStateOf(false)
        var selected by mutableStateOf(false)
        var hasReactions by mutableStateOf(true)
        var followUpDelivery by mutableStateOf<EntryDelivery?>(null)
        var editedFollowUp by mutableStateOf(false)
        var openPicker = 0
        ui.setContent {
            val post = TimelineItem(
                "exhibit", "はる", "展示の準備ができました。明日は10時からです。", "12:05",
                canReact = true, canRepost = true,
                postHeader = PostHeaderData("@haru", "11/28 12:05", PostAudience.Public, null, "zli.chat"),
                reactions = if (!hasReactions) emptyList() else listOf(
                    TimelineReaction("👏", "👏", if (selected) "4" else "3", selected),
                    TimelineReaction("🎉", "🎉", "2", false),
                ),
            )
            val followUp = TimelineItem("follow-up", "はる", "入口でお待ちしています。", "12:07",
                canReact = followUpDelivery == null, continuesGroup = true, delivery = followUpDelivery,
                edited = editedFollowUp)
            val screen = if (channel) AndroidScreen.Conversation("dev", "開発", "", listOf(post, followUp), InputState(""))
                else AndroidScreen.Home(listOf(post))
            AndroidScreens(screen, dark, foreground = true, dispatch = { action ->
                if (action is AndroidUiAction.ActOnEntry && action.operation == EntryOperation.React) {
                    when (action.value) {
                        "👏" -> {
                            check(action.item.id == post.id)
                            selected = !selected
                        }
                        "" -> {
                            check(action.item.id == if (channel) followUp.id else post.id)
                            openPicker++
                        }
                        else -> error("Unexpected reaction")
                    }
                }
            }, language = "ja") { _, _ -> }
        }
        for (room in listOf(false, true)) {
            for (night in listOf(false, true)) {
                ui.runOnIdle { channel = room; dark = night }
                val prefix = "reaction-" + (if (room) "channel" else "home") + (if (night) "-dark" else "-light")
                val chip = ui.onNode(hasText("👏") and hasClickAction())
                fun snapshot(state: String) {
                    capture("$prefix-$state")
                    val rect = ui.onNodeWithTag("timeline-entry-exhibit", useUnmergedTree = true)
                        .fetchSemanticsNode().boundsInRoot
                    val dir = File("build/reports/android-ui")
                    val full = android.graphics.BitmapFactory.decodeFile(dir.resolve("$prefix-$state.png").path)
                    val bitmap = Bitmap.createBitmap(full, rect.left.toInt(), rect.top.toInt(), rect.width.toInt(), rect.height.toInt())
                    dir.resolve("$prefix-$state-detail.png").outputStream().use {
                        check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it))
                    }
                    bitmap.recycle()
                    full.recycle()
                }
                if (!room) {
                    val controls = listOf("返信", "リノート", "リアクションを追加", "その他の投稿操作")
                    fun positions() = controls.map { label ->
                        ui.onNodeWithContentDescription(label).assertHasClickAction().getUnclippedBoundsInRoot().left
                    }
                    ui.runOnIdle { hasReactions = false }
                    val emptyPositions = positions()
                    snapshot("empty")
                    ui.onNodeWithContentDescription("リアクションを追加").performClick()
                    ui.runOnIdle { hasReactions = true }
                    assertEquals(emptyPositions, positions())
                } else {
                    ui.onNodeWithText("はる").assertIsDisplayed()
                    ui.onNodeWithText("12:05").assertIsDisplayed()
                    ui.onNodeWithText("12:07").assertDoesNotExist()
                    val firstBody = ui.onNodeWithText("展示の準備ができました。明日は10時からです。", useUnmergedTree = true)
                    val nextBody = ui.onNodeWithText("入口でお待ちしています。", useUnmergedTree = true)
                    assertEquals(firstBody.getUnclippedBoundsInRoot().left, nextBody.getUnclippedBoundsInRoot().left)
                    ui.onNodeWithContentDescription("リアクションを追加").assertDoesNotExist()
                    val postedBounds = nextBody.getUnclippedBoundsInRoot()
                    ui.runOnIdle { followUpDelivery = EntryDelivery.Sending }
                    assertEquals(postedBounds, nextBody.getUnclippedBoundsInRoot())
                    ui.onNodeWithTag("timeline-entry-follow-up")
                        .assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "送信中"))
                    capture("sending-channel-${if (night) "dark" else "light"}")
                    ui.runOnIdle { followUpDelivery = EntryDelivery.AwaitingSync }
                    ui.onNodeWithText("送信済み").assertIsDisplayed()
                    ui.onNodeWithTag("timeline-entry-follow-up")
                        .assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "送信済み"))
                    ui.runOnIdle {
                        followUpDelivery = null
                        editedFollowUp = true
                    }
                    val time = ui.onNodeWithText("12:07").getUnclippedBoundsInRoot()
                    val pencil = ui.onNodeWithContentDescription("編集済み").getUnclippedBoundsInRoot()
                    check(pencil.left >= time.right)
                    ui.onNodeWithText("はる").assertIsDisplayed()
                    capture("edited-channel-${if (night) "dark" else "light"}")
                    ui.runOnIdle { editedFollowUp = false }
                }
                chip.assertIsNotSelected().assertTextContains("3")
                snapshot("before")
                chip.performClick()
                chip.assertIsSelected().assertTextContains("4")
                snapshot("after")
                chip.performClick()
                chip.assertIsNotSelected().assertTextContains("3")
                if (room) {
                    ui.onNodeWithTag("timeline-entry-follow-up").performTouchInput { longClick() }
                    capture("channel-reaction-menu-${if (night) "dark" else "light"}", withSheet = true)
                    ui.onNodeWithText("リアクション").performClick()
                } else {
                    ui.onNodeWithContentDescription("リアクションを追加").performClick()
                }
            }
        }
        ui.runOnIdle { assertEquals(6, openPicker) }
    }

    @Test
    fun captureAttachmentExport() {
        var share: Boolean? = null
        ui.setContent { AttachmentExportDialog("展示の案内.pdf", "1.5 MB", {}, { share = it }) }
        capture("attachment-save-share", withSheet = true)
        ui.onNodeWithText("共有").performClick()
        ui.runOnIdle { assertEquals(true, share) }
        ui.onNodeWithText("保存").performClick()
        ui.runOnIdle { assertEquals(false, share) }
    }

    @Test
    fun captureSettingsNavigationAndReducedMotion() {
        val settings = AndroidScreen.Settings(listOf(SettingsGroup("外観", listOf(SettingsEntry.Destination("theme", "テーマ", "ライト")))))
        val detail = AndroidScreen.SettingsDetail("テーマ", listOf(SettingsGroup("", listOf(
            SettingsEntry.Choice("light", "ライト", true), SettingsEntry.Choice("dark", "ダーク", false)))))
        var screen by mutableStateOf<AndroidScreen>(settings)
        var reduced by mutableStateOf(false)
        ui.setContent {
            AndroidScreens(screen, false, true, { action ->
                screen = if (action == AndroidUiAction.Back) settings else detail
            }, interaction = AndroidInteractionSettings(reducedMotion = reduced)) { _, _ -> }
        }
        ui.mainClock.autoAdvance = false
        try {
            capture("motion-00")
            ui.onNodeWithText("テーマ").performClick()
            for (frame in 1..14) {
                ui.mainClock.advanceTimeByFrame()
                capture("motion-" + frame.toString().padStart(2, '0'))
            }
            ui.onNodeWithText("ダーク").assertIsDisplayed()
            ui.runOnIdle { screen = settings }
            for (frame in 15..28) {
                ui.mainClock.advanceTimeByFrame()
                capture("motion-" + frame.toString().padStart(2, '0'))
            }
            ui.runOnIdle { reduced = true; screen = detail }
            ui.mainClock.advanceTimeBy(32)
            ui.onNodeWithText("ダーク").assertIsDisplayed()
            ui.onNodeWithText("外観").assertDoesNotExist()
        } finally { ui.mainClock.autoAdvance = true }
    }

    @Test
    fun shareDestinationPreviewUsesTheSameShellInBothThemes() {
        var dark by mutableStateOf(false)
        var destination: String? = null
        ui.setContent {
            AndroidScreens(AndroidScreen.Home(listOf(TimelineItem("entry", "みずき", "週末の展示に向けて準備しています。", "12:05"))),
                dark, true, {}) { _, _ -> }
            ShareDestinationSheet("週末の展示会場はこちらです。", 1, listOf(
                ChannelSection("参加中", listOf(ChannelItem("dev", "開発"), ChannelItem("gallery", "展示準備"))),
                ChannelSection("ダイレクトメッセージ", listOf(ChannelItem("dm", "はる", direct = true)))),
                AndroidColors(dark), false, null, {}, preview = {
                    androidx.compose.foundation.Image(remember { galleryMap() }, "会場案内", Modifier.size(96.dp, 66.dp))
                }, select = { destination = it })
        }
        capture("share-destination-light", bottomDialog = true)
        ui.runOnIdle { dark = true }
        ui.waitForIdle()
        capture("share-destination-dark", bottomDialog = true)
        ui.onNodeWithText("開発").performClick()
        ui.runOnIdle { assertEquals("dev", destination) }
    }

    @Test
    fun shareSearchStaysAvailableAtTheEndOfALargeList() {
        val sections = listOf(
            ChannelSection("参加中", (1..120).map { ChannelItem("c$it", listOf("お知らせ", "会場設営", "受付", "広報", "作品搬入").getOrNull(it - 1) ?: "展示チーム $it") }),
            ChannelSection("ダイレクトメッセージ", (1..80).map {
                if (it == 8 || it == 80) ChannelItem("dm$it", "はる", direct = true, handle = "@haru$it")
                else ChannelItem("dm$it", "メンバー $it", direct = true, handle = "@member$it")
            }),
        )
        var selected: String? = null
        ui.setContent {
            ShareDestinationSheet("週末の展示会場はこちらです。", 0, sections, AndroidColors(false), false, null, {},
                select = { selected = it })
        }
        capture("share-many-destinations", bottomDialog = true)
        ui.onNodeWithTag("share-targets").performScrollToKey("channel-dm80")
        ui.onNodeWithText("@haru80").assertIsDisplayed()
        val search = ui.onNodeWithTag("share-search").assertIsDisplayed()
        capture("share-list-end", bottomDialog = true)
        ui.onNodeWithText("DM").performClick()
        ui.onNodeWithText("@member1").assertIsDisplayed()
        ui.onNodeWithText("ホーム").assertDoesNotExist()
        search.performTextInput("@HARU8")
        ui.onNodeWithText("@haru8").assertIsDisplayed()
        capture("share-destination-results", bottomDialog = true)
        ui.onNodeWithText("@haru80").performClick()
        ui.runOnIdle { assertEquals("dm80", selected) }
        search.performTextReplacement("見つからない宛先")
        ui.onNodeWithText("共有先が見つかりません").assertIsDisplayed()
        ui.onNodeWithContentDescription("検索をクリア").performClick()
        ui.onNodeWithText("すべて").performClick()
        ui.onNodeWithText("ホーム").assertIsDisplayed()
    }

    @Test
    fun channelLoadingDetailAppearsAfterFiveSecondsAndFollowsTheCurrentWait() {
        val starting = "メッセージの同期開始を待っています"
        val retrieving = "最初のメッセージ一覧を取得しています"
        val opening = "チャンネル情報を読み込んでいます"
        val waiting = AndroidScreen.Conversation("dev", "開発", "", emptyList(), InputState(),
            historyReady = false, unreadPositionPending = true, loadingDetail = starting)
        var screen by mutableStateOf<AndroidScreen>(
            AndroidScreen.OpeningConversation("channel:dev", "開発", online = true, loadingDetail = opening),
        )
        ui.mainClock.autoAdvance = false
        try {
            ui.setContent { AndroidScreens(screen, false, true, {}) { _, _ -> } }
            ui.mainClock.advanceTimeBy(3_800)
            ui.onNodeWithText(opening).assertDoesNotExist()
            ui.runOnIdle { screen = waiting }
            ui.mainClock.advanceTimeBy(1_000)
            ui.onNodeWithText(starting).assertDoesNotExist()
            ui.mainClock.advanceTimeBy(400)
            ui.onNodeWithText(starting).assertIsDisplayed()

            ui.runOnIdle { screen = waiting.copy(loadingDetail = retrieving) }
            ui.mainClock.advanceTimeByFrame()
            ui.onNodeWithText(starting).assertDoesNotExist()
            ui.onNodeWithText(retrieving).assertIsDisplayed()

            ui.runOnIdle { screen = waiting.copy(historyReady = true, unreadPositionPending = false, loadingDetail = null) }
            ui.mainClock.advanceTimeByFrame()
            ui.onNodeWithText(retrieving).assertDoesNotExist()
            ui.onNodeWithText(starting).assertDoesNotExist()
        } finally {
            ui.mainClock.autoAdvance = true
        }
    }

    @Test
    @Config(sdk = [35], qualifiers = "ja-rJP-w320dp-h640dp-xhdpi")
    fun narrowScreenWithEnlargedTypeOffersRecovery() {
        var retries = 0
        var screen by mutableStateOf<AndroidScreen>(
            AndroidScreen.OpeningConversation("dev", "開発", online = true, failed = true) { retries++ }
        )
        ui.setContent {
            val density = androidx.compose.ui.platform.LocalDensity.current
            CompositionLocalProvider(
                androidx.compose.ui.platform.LocalDensity provides androidx.compose.ui.unit.Density(density.density, 1.8f)
            ) {
                AndroidScreens(screen, dark = false, foreground = true, dispatch = {}, language = "ja") { _, _ -> }
            }
        }
        ui.waitForIdle()
        capture("opening-conversation-error-large")
        ui.onAllNodes(hasClickAction() and hasText("再試行", substring = true)).onFirst().performClick()
        ui.runOnIdle {
            assertEquals(1, retries)
            screen = AndroidScreen.PostLoading(online = false, failed = true)
        }
        ui.waitForIdle()
        capture("post-offline-large")
    }

    private fun galleryMap(): ImageBitmap {
        val bitmap = Bitmap.createBitmap(420, 236, Bitmap.Config.ARGB_8888)
        bitmap.eraseColor(0xffe9e2cb.toInt())
        val canvas = Canvas(bitmap)
        val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        paint.color = 0xff9ebc98.toInt()
        canvas.drawRoundRect(26f, 24f, 168f, 116f, 10f, 10f, paint)
        paint.color = 0xff86b6c5.toInt()
        paint.strokeWidth = 31f
        canvas.drawLine(300f, -20f, 205f, 255f, paint)
        paint.color = android.graphics.Color.WHITE
        paint.strokeWidth = 14f
        canvas.drawLine(0f, 192f, 420f, 64f, paint)
        canvas.drawLine(45f, 0f, 143f, 236f, paint)
        paint.color = 0xffa84030.toInt()
        canvas.drawCircle(259f, 114f, 15f, paint)
        paint.color = android.graphics.Color.WHITE
        canvas.drawCircle(259f, 114f, 5f, paint)
        return bitmap.asImageBitmap()
    }

    @Test
    fun recoveryFieldMasksInputAndRespectsDisabledState() {
        var value by mutableStateOf("")
        var enabled by mutableStateOf(true)
        ui.setContent { RecoveryKeyField("リカバリーキー", value, enabled, AndroidColors(false)) { value = it } }
        val field = ui.onNodeWithContentDescription("リカバリーキー")
        field.assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.Password))
        field.performTextInput("sample")
        ui.runOnIdle { assertEquals("sample", value); enabled = false }
        field.assertIsNotEnabled()
    }

    private fun captureMenu(name: String) {
        ui.runOnIdle {
            val popup = android.view.inspector.WindowInspector.getGlobalWindowViews().last { it.javaClass.simpleName == "PopupLayout" }
            val bitmap = Bitmap.createBitmap(popup.width, popup.height, Bitmap.Config.ARGB_8888)
            try {
                popup.draw(Canvas(bitmap))
                val out = File("build/reports/android-ui").apply { mkdirs() }
                out.resolve("$name.png").outputStream().use { check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) }
            } finally { bitmap.recycle() }
        }
    }

    private fun capture(name: String, withSheet: Boolean = false, bottomDialog: Boolean = false) {
        if (!name.startsWith("motion-")) ui.recordControlBounds(name)
        ui.runOnIdle {
            val view = ui.activity.findViewById<View>(android.R.id.content)
            val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
            try {
                val canvas = Canvas(bitmap)
                view.draw(canvas)
                if (withSheet || bottomDialog) {
                    val window = checkNotNull(ShadowDialog.getLatestDialog()?.window)
                    val dialog = window.decorView
                    if (bottomDialog) {
                        // Robolectric exposes dialog decor before WindowManager positions it.
                        canvas.translate((view.width - dialog.width) / 2f, (view.height - dialog.height).toFloat())
                    } else {
                        val location = IntArray(2)
                        dialog.getLocationOnScreen(location)
                        canvas.translate(location[0].toFloat(), location[1].toFloat())
                    }
                    dialog.draw(canvas)
                }
                val out = File("build/reports/android-ui").apply { mkdirs() }
                out.resolve("$name.png").outputStream().use {
                    check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it))
                }
            } finally { bitmap.recycle() }
        }
    }
}
