package zoo.android.ui

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Paint
import android.view.View
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import java.io.File
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
                SettingsEntry.Destination("notifications", "通知"), SettingsEntry.Destination("update", "アプリの更新"))),
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
            posts, InputState("確認しました。"))
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
            repostedBy = "みずき",
            quote = TimelineQuote("https://example.test/notes/42", null, "なぎ", "引用元の投稿を共有します。", emptyList()),
            media = listOf(TimelineMedia("post-image", "投稿画像", TimelineMediaKind.Image),
                TimelineMedia("post-video", "投稿動画", TimelineMediaKind.Video, sensitive = true)),
        )
        val illustratedMessage = posts.first().copy(media = listOf(
            TimelineMedia("channel-image", "添付画像", TimelineMediaKind.Image, attachmentId = "channel-image"),
            TimelineMedia("channel-video", "添付動画", TimelineMediaKind.Video, attachmentId = "channel-video"),
        ))
        val richPost = homePosts.first().copy(
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
        ))
        val cases = listOf(
            "home-rich" to AndroidScreen.Home(listOf(richPost)),
            "channel-rich" to conversation.copy(entries = listOf(richMessage), input = InputState()),
            "home" to AndroidScreen.Home(homePosts),
            "home-media" to AndroidScreen.Home(listOf(illustratedPost)),
            "channel-media" to conversation.copy(entries = listOf(illustratedMessage), input = InputState()),
            "channels" to channels,
            "notifications" to inbox,
            "bubble" to AndroidScreen.Bubble(posts.take(2), InputState(live = true), listOf("はる", "ゆい")),
            "settings" to AndroidScreen.Settings(settings),
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
            "search-error" to AndroidScreen.Search(SearchScreenState("議事録", "議事録", GlobalSearchType.All, error = true)),
            "channel-edit" to AndroidScreen.EditChannel("チャンネルを作成", "開発", "", "作成", true,
                privateChannel = true, sensitive = false, encrypted = true),
            "channel-settings" to AndroidScreen.ChannelSettings("開発", listOf(SettingsGroup("チャンネル", listOf(
                SettingsEntry.Information("description", "Zoo の開発と通話のテストについて相談するチャンネルです。"),
                SettingsEntry.Destination("members", "メンバー"),
                SettingsEntry.Destination("add-member", "メンバーを追加"),
                SettingsEntry.Destination("rename", "名前と説明を編集"),
                SettingsEntry.Destination("archive", "アーカイブ"),
            )))),
            "channel-members" to AndroidScreen.Directory("メンバー", "", listOf(
                SelectionItem("haru", "はる", isUserName = true),
                SelectionItem("yui", "ゆい", isUserName = true),
            )),
            "channel-add-member" to AndroidScreen.Directory("メンバーを追加", "", listOf(
                SelectionItem("mizuki", "みずき", isUserName = true),
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
        ui.setContent {
            CompositionLocalProvider(LocalTimelineImage provides { if (it.key == "fixture-emoji") reactionEmoji else avatar }) {
                AndroidScreens(screen, dark, foreground || screen is AndroidScreen.Call, dispatch = { action ->
                    actions.add(action)
                    if (action == AndroidUiAction.UseInputTool(InputTool.Emoji)) emojiPicker = true
                    if (action == AndroidUiAction.OpenChannelSettings("dev")) {
                        screen = AndroidScreen.ChannelSettings("開発", settings)
                    }
                }, language = "ja", remoteAudioLevel = { .4f }, loadNotificationAvatar = { if (it == "fixture-emoji") reactionEmoji else avatar }) { item, modifier ->
                    Box(modifier.background(Color.Gray), contentAlignment = Alignment.Center) { Text(item.label) }
                }
            }
            if (emojiPicker) {
                val choices = remember {
                    listOf("👍", "😊", "🎉", "❤️", "😀", "😂", "🥰", "😎", "🥳", "🙌", "👏", "👋", "✨", "🔥",
                        "🌈", "🚀", "🌍", "🍕", "🍩", "☕", "⚽", "🎮", "🎵", "🎁", "🎂", "💡", "💯", "✅",
                        "🐶", "🦊", "🐼", "🐻", "🐸", "🐵", "🦁", "🐯", "🐧", "🦄", "🦋", "🐙", "🌸", "🌻")
                        .map { EmojiChoice(it, it, it) } + EmojiChoice("🐱", "🐱", "cat", keywords = "ねこ")
                }
                EmojiPicker("絵文字", AndroidColors(dark), choices,
                    custom = listOf(EmojiChoice(":party:", "", "party", "fixture-emoji")),
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
                }
                if (name == "channels") {
                    for (label in listOf("チャンネルを作成", "DMを作成")) {
                        ui.onNodeWithContentDescription(label).assertHasClickAction()
                            .assertWidthIsEqualTo(48.dp).assertHeightIsEqualTo(48.dp)
                    }
                }
                if (name == "notifications") {
                    ui.onNodeWithText("未読").assertDoesNotExist()
                    ui.onNodeWithText("リアクション · 2件").assertIsDisplayed()
                    ui.onNodeWithText("🎉").assertIsDisplayed()
                    ui.onNodeWithContentDescription(":star:").assertIsDisplayed()
                    ui.onNodeWithContentDescription("みずき", useUnmergedTree = true).assertIsDisplayed()
                    ui.onNodeWithContentDescription("なぎ", useUnmergedTree = true).assertIsDisplayed()
                    ui.onAllNodesWithContentDescription("未読", useUnmergedTree = true).assertCountEquals(2)
                }
                if (name == "home-media") {
                    val repost = ui.onNodeWithText("みずきさんが再投稿", useUnmergedTree = true)
                        .assertIsDisplayed().getUnclippedBoundsInRoot()
                    val author = ui.onNodeWithText("@haru", useUnmergedTree = true).getUnclippedBoundsInRoot()
                    check(repost.left < author.left && repost.bottom <= author.top)
                    ui.onNodeWithText("引用 · なぎ").assertIsDisplayed().performClick()
                    ui.onNodeWithText("センシティブな動画を開く").assertIsDisplayed().performClick()
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
                    ui.onNodeWithContentDescription("公開", useUnmergedTree = true).assertIsDisplayed()
                    val channelBounds = ui.onNodeWithText("#開発", useUnmergedTree = true)
                        .assertIsDisplayed().getUnclippedBoundsInRoot()
                    check(channelBounds.bottom - channelBounds.top == serverBounds.bottom - serverBounds.top)
                    val channelHandle = ui.onNodeWithText("@mizuki", useUnmergedTree = true).getUnclippedBoundsInRoot()
                    check(channelBounds.top - channelHandle.bottom == serverBounds.top - nameBounds.bottom)
                    val firstPost = hasAnyAncestor(hasTestTag("timeline-entry-1"))
                    val controls = listOf("返信", "再投稿", "リアクションを追加", "その他の投稿操作").map { label ->
                        ui.onNode(firstPost and hasContentDescription(label), useUnmergedTree = true)
                            .assertHasClickAction().assertIsEnabled().assertHeightIsEqualTo(36.dp).getUnclippedBoundsInRoot()
                    }
                    val postBounds = ui.onNodeWithTag("timeline-entry-1", useUnmergedTree = true).getUnclippedBoundsInRoot()
                    val bodyBounds = ui.onNode(firstPost and hasText(homePosts.first().body), useUnmergedTree = true)
                        .getUnclippedBoundsInRoot()
                    check(controls.first().left == bodyBounds.left && controls.last().right == postBounds.right)
                    val gaps = controls.zipWithNext { left, right -> (right.left - left.right).value }
                    check(gaps.max() - gaps.min() <= 1f)
                    ui.onNode(hasAnyAncestor(hasTestTag("timeline-entry-2")) and hasContentDescription("再投稿"),
                        useUnmergedTree = true).assertIsNotEnabled()
                }
                if (name == "channels") {
                    ui.onNodeWithContentDescription("検索").assertHasClickAction()
                        .assertWidthIsEqualTo(48.dp).assertHeightIsEqualTo(48.dp)
                }
                if (name == "search") {
                    ui.onNodeWithText("暗号化されたチャンネルは検索の対象外です。").assertIsDisplayed()
                    // Terms are marked in the snippet's text and in file names, but not inside code.
                    fun marked(prefix: String) = ui.onNode(hasText(prefix, substring = true), useUnmergedTree = true)
                        .fetchSemanticsNode().config[SemanticsProperties.Text].single().let { text ->
                            text.spanStyles.filter { it.item.background != Color.Unspecified }
                                .map { text.text.substring(it.start, it.end) }
                        }
                    check(marked("…次の会議") == listOf("会議", "資料")) { marked("…次の会議").toString() }
                    check(marked("会議資料_2026") == listOf("会議", "資料")) { marked("会議資料_2026").toString() }
                    ui.onNodeWithText("すべて").assertIsSelected()
                    ui.onNodeWithText("運営 · スレッドの返信", useUnmergedTree = true).assertIsDisplayed()
                    ui.onNodeWithText("1.2 MB", useUnmergedTree = true).assertIsDisplayed()
                    ui.onNodeWithTag("global-search-results").performScrollToNode(hasText("さらに古い結果を検索"))
                    ui.onNodeWithText("2025-12 まで検索しました").assertIsDisplayed()
                    ui.onNodeWithText("さらに古い結果を検索").assertHeightIsEqualTo(48.dp)
                    ui.onNodeWithTag("global-search-results").performScrollToIndex(0)
                }
                if (name == "search-files") {
                    ui.onNodeWithText("ファイル").assertIsSelected()
                    ui.onNodeWithText("2025-12 まで検索しました").assertIsDisplayed()
                    ui.onNodeWithText("さらに古い結果を検索").assertDoesNotExist()
                }
                if (name == "search-older") {
                    ui.onNodeWithText("2025-12 までに見つかりませんでした。さらに古い期間を検索できます。").assertIsDisplayed()
                    ui.onNodeWithText("さらに古い結果を検索").assertIsDisplayed()
                }
                if (name == "search-error") ui.onNodeWithText("再試行").assertHasClickAction()
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
                    val audience = ui.onNodeWithContentDescription("公開範囲：公開").assertHasClickAction()
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
                    ui.onNodeWithContentDescription("公開範囲：未収載").assertIsDisplayed()
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
            capture("emoji-picker-${if (night) "dark" else "light"}", bottomDialog = true)
            ui.onNodeWithTag("emoji-search").performTextInput("cat")
            ui.onNodeWithContentDescription("cat").assertIsDisplayed()
            ui.onNodeWithText("🎉").assertDoesNotExist()
            ui.onNodeWithText("カスタム").performClick()
            ui.onNodeWithContentDescription("party").assertIsDisplayed()
            capture("emoji-picker-custom-${if (night) "dark" else "light"}", bottomDialog = true)
            ui.onNodeWithText("Emoji Kitchen").performClick()
            ui.onNodeWithContentDescription("cat").performClick()
            ui.onNodeWithContentDescription("cat duo").assertIsDisplayed()
            capture("emoji-picker-kitchen-${if (night) "dark" else "light"}", bottomDialog = true)
            ui.onNodeWithContentDescription("cat duo").performClick()
        }
        ui.runOnIdle { screen = cases.first { it.first == "post-compose" }.second }
        ui.onNodeWithContentDescription("公開範囲：公開").performClick()
        for (audience in listOf("公開", "未収載", "フォロワー限定", "ローカル")) {
            ui.onNodeWithText(audience).assertIsDisplayed()
        }
        ui.onNodeWithText("未収載").performClick()
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
        for (label in listOf("再投稿", "リアクションを追加", "その他の投稿操作")) {
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
        ui.onNodeWithContentDescription("検索").performClick()
        ui.runOnIdle { screen = searchResults }
        ui.onNodeWithTag("global-search-field").performTextReplacement("会議")
        ui.onNodeWithText("メッセージ").performClick()
        ui.onNodeWithText("みずき", useUnmergedTree = true).performClick()
        ui.onNodeWithTag("global-search-results").performScrollToNode(hasText("さらに古い結果を検索"))
        ui.onNodeWithText("さらに古い結果を検索").performClick()
        ui.runOnIdle {
            check(actions.takeLast(5) == listOf(
                AndroidUiAction.OpenSearch,
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
        ui.onNodeWithText("テーマ").assertIsDisplayed()
        ui.runOnIdle {
            check(actions.contains(AndroidUiAction.OpenChannelSettings("dev")))
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
        ui.onNodeWithText("再投稿").assertIsDisplayed()
        capture("post-menu-light", withSheet = true)
        ui.runOnIdle { dark = true }
        capture("post-menu-dark", withSheet = true)
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

    private fun capture(name: String, withSheet: Boolean = false, bottomDialog: Boolean = false) {
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
