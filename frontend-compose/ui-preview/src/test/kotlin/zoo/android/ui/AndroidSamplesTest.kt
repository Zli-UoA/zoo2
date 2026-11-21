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
            posts[0].copy(postHeader = PostHeaderData("@haru", "11/21 12:05", PostAudience.Public, null, "misskey.io")),
            posts[1].copy(postHeader = PostHeaderData("@mizuki", "11/21 11:56", PostAudience.Local, "開発", "zli.chat")),
            posts[2].copy(postHeader = PostHeaderData("@yui", "11/21 11:42", PostAudience.Unlisted, null, "zli.chat")),
        )
        val settings = listOf(
            SettingsGroup("外観", listOf(SettingsEntry.Destination("theme", "テーマ", "端末に従う"),
                SettingsEntry.Destination("language", "言語", "日本語"))),
            SettingsGroup("操作", listOf(SettingsEntry.Toggle("swipe", "スワイプで返信", true),
                SettingsEntry.Toggle("haptics", "触覚フィードバック", true))),
            SettingsGroup("Zoo", listOf(SettingsEntry.Destination("general", "一般"),
                SettingsEntry.Destination("notifications", "通知"), SettingsEntry.Destination("update", "アプリの更新"))),
        )
        val channels = AndroidScreen.Channels(listOf(
            ChannelSection("参加中", listOf(ChannelItem("dev", "開発", true), ChannelItem("chat", "雑談"),
                ChannelItem("ops", "運営"), ChannelItem("news", "お知らせ"))),
            ChannelSection("ダイレクトメッセージ", listOf(ChannelItem("dm", "はる", isUserName = true))),
        ))
        val inbox = AndroidScreen.Notifications(listOf(
            InboxItem("reply", "はる", "投稿に返信しました", "次のテストでも確認します。", "12:03", true),
            InboxItem("mention", "ゆい", "開発であなたをメンションしました", "ライト版も読みやすいね。", "11:48", true),
            InboxItem("reaction", "みずき", "投稿にリアクションしました", null, "11:22", false),
            InboxItem("follow", "りん", "フォローを申請しました", null, "11:15", false,
                followActor = "https://example.test/rin", followActivity = "https://example.test/follow"),
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
        val cases = listOf(
            "home" to AndroidScreen.Home(homePosts),
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
            "post-compose" to AndroidScreen.NewPost(InputState("新しい画面を共有します。"), emptyList(), true),
            "profile" to AndroidScreen.Profile("はる", "Android 版をテストしています。写真と通話の感想を投稿します。",
                homePosts.map {
                    it.copy(author = "はる", postHeader = it.postHeader?.copy(handle = "@haru", channel = null, server = "social.example"))
                }, "フォロー", handle = "@haru@social.example", avatar = avatar, originalAvailable = true),
            "directory" to AndroidScreen.Directory("チャンネルを探す", "", listOf(SelectionItem("dev", "開発"))),
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
            "call-active" to AndroidScreen.Call("はる", false, active = true, avatar = avatar, hasUser = true),
        )
        var screen by mutableStateOf<AndroidScreen>(cases.first().second)
        var dark by mutableStateOf(false)
        var foreground by mutableStateOf(false)
        val actions = mutableListOf<AndroidUiAction>()
        ui.setContent {
            AndroidScreens(screen, dark, foreground || screen is AndroidScreen.Call, dispatch = { action ->
                actions.add(action)
                if (action == AndroidUiAction.OpenChannelSettings("dev")) {
                    screen = AndroidScreen.ChannelSettings("開発", settings)
                }
            }, language = "ja", remoteAudioLevel = { .4f }) { item, modifier ->
                Box(modifier.background(Color.Gray), contentAlignment = Alignment.Center) { Text(item.label) }
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
                if (name == "home") {
                    val nameBounds = ui.onNodeWithText("@haru", useUnmergedTree = true).getUnclippedBoundsInRoot()
                    val dateBounds = ui.onNodeWithText("11/21 12:05", useUnmergedTree = true).getUnclippedBoundsInRoot()
                    val serverBounds = ui.onNodeWithText("misskey.io", useUnmergedTree = true).getUnclippedBoundsInRoot()
                    check(nameBounds.right < dateBounds.left && serverBounds.top > dateBounds.top)
                    ui.onNodeWithContentDescription("公開", useUnmergedTree = true).assertIsDisplayed()
                    ui.onNodeWithText("#開発", useUnmergedTree = true).assertIsDisplayed()
                }
                if (name == "profile") {
                    ui.onNodeWithText("@haru@social.example").assertIsDisplayed()
                    ui.onNodeWithContentDescription("公開プロフィールを開く").assertHasClickAction()
                    ui.onNodeWithText("フォロー").assertHasClickAction()
                }
                if (name == "post-compose") {
                    val toolbar = ui.onNodeWithTag("post-actions").getUnclippedBoundsInRoot()
                    val post = ui.onNodeWithText("投稿").assertHeightIsEqualTo(48.dp).getUnclippedBoundsInRoot()
                    val photos = ui.onNodeWithContentDescription("写真").getUnclippedBoundsInRoot()
                    check(
                        post.top - toolbar.top == 8.dp && toolbar.bottom - post.bottom == 8.dp &&
                            toolbar.right - post.right == 8.dp && photos.left - toolbar.left == 8.dp,
                    ) { "Expected an 8dp inset around the post actions: $post / $photos / $toolbar" }
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
                }
                if (example is AndroidScreen.Call) {
                    if (example.active) ui.mainClock.advanceTimeBy(150)
                    ui.onNodeWithTag("call-avatar").assertWidthIsEqualTo(88.dp).assertHeightIsEqualTo(88.dp)
                }
                capture("$name-${if (night) "dark" else "light"}")
            }
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

    private fun capture(name: String, withSheet: Boolean = false) {
        ui.runOnIdle {
            val view = ui.activity.findViewById<View>(android.R.id.content)
            val bitmap = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
            try {
                val canvas = Canvas(bitmap)
                view.draw(canvas)
                if (withSheet) {
                    val sheet = checkNotNull(ShadowDialog.getLatestDialog()?.window?.decorView)
                    val location = IntArray(2)
                    sheet.getLocationOnScreen(location)
                    canvas.translate(location[0].toFloat(), location[1].toFloat())
                    sheet.draw(canvas)
                }
                val out = File("build/reports/android-ui").apply { mkdirs() }
                out.resolve("$name.png").outputStream().use {
                    check(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it))
                }
            } finally { bitmap.recycle() }
        }
    }
}
