import XCTest

final class ZooUITests: XCTestCase {
    @MainActor
    func testCallSurvivesBackgroundAndCanEndAfterReturning() {
        let app = launchWorkspace(call: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        app.buttons["start-call"].tap()
        let mute = app.buttons["call-mute"]
        XCTAssertTrue(mute.waitForExistence(timeout: 5))
        XCTAssertTrue(mute.isEnabled)
        mute.tap()
        XCTAssertEqual(mute.label, "ミュートを解除")
        XCUIDevice.shared.press(.home)
        XCTAssertTrue(app.wait(for: .runningBackground, timeout: 5) || app.state == .runningBackgroundSuspended)
        app.activate()
        XCTAssertTrue(mute.waitForExistence(timeout: 5))
        XCTAssertEqual(mute.label, "ミュートを解除")
        XCTAssertFalse(app.staticTexts["通話は終了しました。"].exists)
        mute.tap()
        XCTAssertEqual(mute.label, "ミュート")
        capture(app, name: "バックグラウンドから復帰した通話")
        app.buttons["閉じる"].tap()
        let banner = app.buttons["active-call"]
        XCTAssertTrue(banner.waitForExistence(timeout: 5))
        banner.tap()
        app.buttons["call-end"].tap()
        XCTAssertTrue(banner.waitForNonExistence(timeout: 5))
    }

    @MainActor
    func testChannelPinsFromDirectoryAndDetails() {
        let app = launchWorkspace()
        let channel = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        channel.press(forDuration: 0.8)
        let pin = app.buttons["toggle-channel-pin"]
        XCTAssertTrue(pin.waitForExistence(timeout: 5))
        XCTAssertEqual(pin.label, "ピン留め")
        capture(app, name: "一覧の長押しでピン留め")
        pin.tap()
        XCTAssertTrue(app.staticTexts["ピン留め"].waitForExistence(timeout: 5))
        XCTAssertEqual(app.buttons.matching(identifier: channel.identifier).count, 1)
        channel.tap()
        let header = app.buttons["channel-details"]
        XCTAssertTrue(header.waitForExistence(timeout: 5))
        header.tap()
        XCTAssertTrue(app.navigationBars["チャンネル詳細"].waitForExistence(timeout: 5))
        let toggle = app.switches["channel-pin-toggle"]
        XCTAssertTrue(toggle.waitForExistence(timeout: 5))
        XCTAssertEqual(toggle.value as? String, "1")
        capture(app, name: "チャンネル詳細のピン留め設定")
        toggle.switches.firstMatch.tap()
        let unpinned = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@ AND enabled == true", "0"), object: toggle)
        XCTAssertEqual(XCTWaiter.wait(for: [unpinned], timeout: 5), .completed)
        toggle.switches.firstMatch.tap()
        let pinned = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@ AND enabled == true", "1"), object: toggle)
        XCTAssertEqual(XCTWaiter.wait(for: [pinned], timeout: 5), .completed)
        toggle.switches.firstMatch.tap()
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@ AND enabled == true", "0"), object: toggle)], timeout: 5), .completed)
        app.buttons["閉じる"].tap()
        app.navigationBars.buttons.firstMatch.tap()
        XCTAssertTrue(channel.waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["ピン留め"].exists)
        channel.press(forDuration: 0.8)
        XCTAssertTrue(pin.waitForExistence(timeout: 5))
        XCTAssertEqual(pin.label, "ピン留め")
    }

    @MainActor
    func testPinFailureStaysVisibleAboveLongDirectory() {
        let app = launchWorkspace(pinFailure: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].press(forDuration: 0.8)
        let pin = app.buttons["toggle-channel-pin"]
        XCTAssertTrue(pin.waitForExistence(timeout: 5))
        pin.tap()
        let error = app.staticTexts["ピン留めを変更できませんでした。接続を確認して、もう一度お試しください。"]
        XCTAssertTrue(error.waitForExistence(timeout: 5))
        XCTAssertTrue(error.isHittable)
        app.swipeUp()
        XCTAssertTrue(error.isHittable)
        capture(app, name: "長い一覧の上部に固定したピン保存エラー")
    }

    @MainActor
    func testPinFailureCanBeRetriedInsideDetails() {
        let app = launchWorkspace(pinFailure: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        app.buttons["channel-details"].tap()
        let pin = app.switches["channel-pin-toggle"]
        XCTAssertTrue(pin.waitForExistence(timeout: 5))
        pin.switches.firstMatch.tap()
        let error = app.staticTexts["ピン留めを変更できませんでした。接続を確認して、もう一度お試しください。"]
        XCTAssertTrue(error.waitForExistence(timeout: 5))
        XCTAssertEqual(pin.value as? String, "0")
        capture(app, name: "詳細画面のピン保存エラー")
        pin.switches.firstMatch.tap()
        let saved = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@ AND enabled == true", "1"), object: pin)
        XCTAssertEqual(XCTWaiter.wait(for: [saved], timeout: 5), .completed)
        XCTAssertFalse(error.exists)
    }

    @MainActor
    func testPinnedChannelDetailsAndMentionSuggestions() {
        let app = launchWorkspace(chatDetails: true)
        let channel = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        XCTAssertEqual(app.buttons.matching(identifier: channel.identifier).count, 1)
        XCTAssertTrue(app.staticTexts["ピン留め"].exists)
        capture(app, name: "ピン留めと一覧")
        channel.tap()
        XCTAssertTrue(app.staticTexts["ここから未読"].waitForExistence(timeout: 10))
        app.buttons["channel-details"].tap()
        XCTAssertTrue(app.navigationBars["チャンネル詳細"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["@sakura"].waitForExistence(timeout: 5))
        capture(app, name: "チャンネル詳細とメンバー")
        app.buttons["閉じる"].tap()
        let field = composer(app)
        field.tap()
        field.typeText("@sak")
        XCTAssertTrue(app.buttons["mention-sakura"].waitForExistence(timeout: 5))
        capture(app, name: "メンション入力候補")
        app.buttons["mention-sakura"].tap()
        XCTAssertEqual(field.value as? String, "@sakura ")
        field.typeText("確認をお願いします")
        app.buttons["send-message"].tap()
        XCTAssertTrue(app.staticTexts["@sakura 確認をお願いします"].waitForExistence(timeout: 5))
    }

    @MainActor
    func testReactionLongPressShowsPeopleWithoutToggling() {
        let app = launchWorkspace()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let reaction = app.buttons["reaction-22222222-2222-4222-8222-222222222222-👍"]
        XCTAssertTrue(reaction.waitForExistence(timeout: 10))
        reaction.press(forDuration: 0.8)
        XCTAssertTrue(app.staticTexts["1人がリアクションしました"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.navigationBars["リアクションした人"].exists)
        XCTAssertFalse(app.buttons["閉じる"].exists)
        XCTAssertTrue(app.staticTexts["@sakura"].exists)
        XCTAssertFalse(app.buttons["message-action-more-reactions"].exists)
        capture(app, name: "リアクションしたメンバー")
        app.descendants(matching: .any)["reaction-people-sheet"].swipeDown()
        XCTAssertEqual(reaction.value as? String, "1人")
        reaction.tap()
        let selected = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "2人、自分も付けています"), object: reaction)
        XCTAssertEqual(XCTWaiter.wait(for: [selected], timeout: 5), .completed)
    }

    @MainActor
    func testImageAttachmentPreviewAndDateDividers() {
        let app = launchWorkspace(chatDetails: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let preview = app.buttons["画像を拡大：さくら.png"]
        XCTAssertTrue(preview.waitForExistence(timeout: 10))
        capture(app, name: "未読線・日付・メンション・画像")
        preview.tap()
        XCTAssertTrue(app.navigationBars["さくら.png"].waitForExistence(timeout: 5))
        capture(app, name: "添付画像の拡大")
        app.buttons["閉じる"].tap()
        XCTAssertTrue(app.descendants(matching: .any)["date-divider-22222222-2222-4222-8222-222222222222"].exists)
        XCTAssertTrue(app.descendants(matching: .any)["date-divider-66666666-6666-4666-8666-666666666666"].exists)
    }

    @MainActor
    func testVideoAttachmentPlayback() {
        let app = launchWorkspace(video: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let play = app.buttons["動画を再生：確認用.mp4"]
        XCTAssertTrue(play.waitForExistence(timeout: 10))
        play.tap()
        XCTAssertTrue(app.navigationBars["確認用.mp4"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.descendants(matching: .any)["attachment-video-player"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["添付を開けません"].exists)
        capture(app, name: "添付動画の再生")
        app.buttons["閉じる"].tap()
        XCTAssertTrue(play.waitForExistence(timeout: 5))
    }

    @MainActor
    func testReadStateRetryMarksShortConversationRead() {
        let app = launchWorkspace(readRetry: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        XCTAssertTrue(app.staticTexts["未読の位置を取得できませんでした。"].waitForExistence(timeout: 10))
        app.buttons["再読み込み"].tap()
        XCTAssertTrue(app.staticTexts["ここから未読"].waitForExistence(timeout: 5))
        capture(app, name: "取得失敗から復旧した未読境界")
        app.navigationBars.buttons.firstMatch.tap()
        app.buttons["未読のみ"].tap()
        XCTAssertTrue(app.staticTexts["未読の会話はありません"].waitForExistence(timeout: 5))
    }

    @MainActor
    func testComposerToolbarInsertsMentionAndEmoji() {
        let app = launchWorkspace(dark: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let field = composer(app)
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        capture(app, name: "Slackに合わせた通常時の入力欄")
        field.tap()
        app.buttons["composer-mention"].tap()
        XCTAssertTrue(app.buttons["mention-sakura"].waitForExistence(timeout: 5))
        capture(app, name: "Slackに合わせた入力欄と候補パネル")
        app.buttons["mention-sakura"].tap()
        field.typeText("ありがとう ")
        app.buttons["composer-emoji"].tap()
        XCTAssertTrue(app.buttons["pick-reaction-🎉"].waitForExistence(timeout: 5))
        app.buttons["pick-reaction-🎉"].tap()
        XCTAssertTrue(app.buttons["send-message"].wait(for: \.isHittable, toEqual: true, timeout: 5))
        XCTAssertEqual(field.value as? String, "@sakura ありがとう 🎉")
        capture(app, name: "スタンプを挿入した入力欄")
        app.buttons["send-message"].tap()
        XCTAssertTrue(app.staticTexts["@sakura ありがとう 🎉"].waitForExistence(timeout: 5))
    }

    @MainActor
    func testWelcomeHasAccessibleLogin() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting"]
        app.launch()
        XCTAssertTrue(app.buttons["login"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["login"].isEnabled)
    }

    @MainActor
    private func launchWorkspace(failingSend: Bool = false, liveMessage: Bool = false, longHistory: Bool = false,
                                 threadDirectory: Bool = false, chatDetails: Bool = false, video: Bool = false, readRetry: Bool = false, dark: Bool = false, pinFailure: Bool = false, call: Bool = false) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace"]
        if failingSend { app.launchArguments.append("--uitesting-send-failure") }
        if liveMessage { app.launchArguments.append("--uitesting-live-message") }
        if longHistory { app.launchArguments.append("--uitesting-long-history") }
        if dark { app.launchArguments += ["-zoo.appearance", "dark"] }
        if video { app.launchArguments.append("--uitesting-video") }
        if readRetry { app.launchArguments.append("--uitesting-read-retry") }
        if pinFailure { app.launchArguments.append("--uitesting-pin-failure") }
        if call { app.launchArguments.append("--uitesting-call") }
        if chatDetails { app.launchArguments.append("--uitesting-chat-details") }
        if threadDirectory { app.launchArguments.append("--uitesting-thread-directory") }
        app.launch()
        XCTAssertTrue(app.buttons["channel-11111111-1111-4111-8111-111111111111"].waitForExistence(timeout: 10))
        return app
    }

    @MainActor
    private func composer(_ app: XCUIApplication, reply: Bool = false) -> XCUIElement {
        let id = reply ? "reply-input" : "message-input"
        let multiline = app.textViews[id]
        if multiline.waitForExistence(timeout: 2) { return multiline }
        return app.textFields[id]
    }

    @MainActor
    private func capture(_ app: XCUIApplication, name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    @MainActor
    private func openActions(for message: XCUIElement, in app: XCUIApplication) {
        XCTAssertTrue(message.waitForExistence(timeout: 5))
        message.press(forDuration: 0.8)
        XCTAssertTrue(app.buttons["message-action-more-reactions"].waitForExistence(timeout: 5))
    }

    @MainActor
    private func selectTab(_ title: String, in app: XCUIApplication, dismissingKeyboard: Bool = false) {
        if dismissingKeyboard {
            let dismiss = app.buttons["dismiss-composer-keyboard"]
            XCTAssertTrue(dismiss.wait(for: \.isHittable, toEqual: true, timeout: 5))
            dismiss.tap()
        }
        let tab = app.tabBars.buttons[title]
        XCTAssertTrue(tab.wait(for: \.isHittable, toEqual: true, timeout: 5))
        tab.tap()
        XCTAssertTrue(tab.wait(for: \.isSelected, toEqual: true, timeout: 5))
    }

    @MainActor
    func testNativeConversationSendsAndReplies() {
        let app = launchWorkspace(liveMessage: true)
        capture(app, name: "チャンネル一覧")
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        XCTAssertTrue(app.staticTexts["こんにちは！ Zoo の iOS 版へようこそ。"].waitForExistence(timeout: 10))
        let field = composer(app)
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        field.tap()
        field.typeText("日本語で送信テスト")
        app.buttons["send-message"].tap()
        XCTAssertTrue(app.staticTexts["日本語で送信テスト"].waitForExistence(timeout: 5))
        XCTAssertEqual(field.value as? String, "#一般 へのメッセージ")
        capture(app, name: "ネイティブチャットと日本語入力")
        openActions(for: app.staticTexts["こんにちは！ Zoo の iOS 版へようこそ。"], in: app)
        app.buttons["message-action-reply"].tap()
        let threadBar = app.navigationBars["スレッド"]
        XCTAssertTrue(threadBar.waitForExistence(timeout: 5))
        XCTAssertFalse(threadBar.buttons["閉じる"].exists)
        app.typeText("スレッドへの返信テスト")
        app.buttons["send-reply"].tap()
        XCTAssertTrue(app.staticTexts["スレッドへの返信テスト"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["別の端末からの返信"].waitForExistence(timeout: 10))
        capture(app, name: "スレッドへの返信")
        threadBar.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(app.navigationBars["一般"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "2 件の返信"))
            .firstMatch.waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["別の端末からの新着"].waitForExistence(timeout: 10))
        XCTAssertFalse(app.navigationBars["スレッド"].exists)
        XCTAssertFalse(app.webViews.firstMatch.exists)
    }

    @MainActor
    func testMessageReactionsCanBeAddedAndRemoved() {
        let app = launchWorkspace()
        let messageID = "22222222-2222-4222-8222-222222222222"
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let welcome = app.staticTexts["message-content-\(messageID)"]
        let thumbsUp = app.buttons.matching(identifier: "reaction-\(messageID)-👍")
        let otherOnly = thumbsUp.matching(NSPredicate(format: "value == %@", "1人")).firstMatch
        let includingMe = thumbsUp.matching(NSPredicate(format: "value == %@", "2人、自分も付けています")).firstMatch
        XCTAssertTrue(otherOnly.waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["reply-\(messageID)"].exists)
        XCTAssertFalse(app.buttons["add-reaction-\(messageID)"].exists)
        openActions(for: welcome, in: app)
        XCTAssertTrue(app.buttons["message-action-reply"].exists)
        XCTAssertFalse(app.buttons["message-action-edit"].exists)
        XCTAssertFalse(app.buttons["message-action-delete"].exists)
        capture(app, name: "長押しで開いたメッセージの操作")
        app.buttons["message-action-quick-👍"].tap()
        XCTAssertTrue(includingMe.waitForExistence(timeout: 5))
        includingMe.tap()
        XCTAssertTrue(otherOnly.waitForExistence(timeout: 5))

        openActions(for: welcome, in: app)
        app.buttons["message-action-more-reactions"].tap()
        let search = app.searchFields.firstMatch
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        search.tap()
        search.typeText("hana")
        let sakura = app.buttons["pick-reaction-:sakura:"]
        XCTAssertTrue(sakura.waitForExistence(timeout: 5))
        sakura.tap()
        let custom = app.buttons["reaction-\(messageID)-:sakura:"]
        XCTAssertTrue(custom.waitForExistence(timeout: 5))
        XCTAssertEqual(custom.value as? String, "1人、自分も付けています")
        custom.tap()
        XCTAssertTrue(custom.waitForNonExistence(timeout: 5))

        openActions(for: welcome, in: app)
        app.buttons["message-action-more-reactions"].tap()
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        search.tap()
        search.typeText("@zoo")
        let account = app.buttons["pick-reaction-:@zoo:"]
        XCTAssertTrue(account.waitForExistence(timeout: 5))
        account.tap()
        let accountChip = app.buttons["reaction-\(messageID)-:@zoo:"]
        XCTAssertTrue(accountChip.waitForExistence(timeout: 5))
        XCTAssertEqual(accountChip.label, "動物園（@zoo）のスタンプ")
        openActions(for: welcome, in: app)
        let recent = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "message-action-quick-"))
        XCTAssertEqual(recent.element(boundBy: 0).identifier, "message-action-quick-:@zoo:")
        XCTAssertEqual(recent.element(boundBy: 1).identifier, "message-action-quick-:sakura:")
        XCTAssertEqual(recent.element(boundBy: 2).identifier, "message-action-quick-👍")
        capture(app, name: "利用履歴順のスタンプ候補")
    }

    @MainActor
    func testMessageLinksAndInlineEmoji() {
        let app = launchWorkspace()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let input = composer(app)
        input.tap()
        let original = "確認 :smile: :hana: :unknown_stamp:\nhttps://example.com/"
        app.typeText(original)
        app.buttons["send-message"].tap()
        let message = app.staticTexts.matching(NSPredicate(format: "identifier BEGINSWITH %@ AND label CONTAINS %@",
                                                          "message-content-", "確認 😄")).firstMatch
        XCTAssertTrue(message.waitForExistence(timeout: 5))
        XCTAssertTrue(message.label.contains(":unknown_stamp:"))
        let link = app.links["https://example.com/"]
        XCTAssertTrue(link.waitForExistence(timeout: 5))
        capture(app, name: "本文のリンクとスタンプ")
        // 本文中央は URL と重なるため、通常の文字部分を長押しする。
        message.coordinate(withNormalizedOffset: CGVector(dx: 0.1, dy: 0.1)).press(forDuration: 0.8)
        XCTAssertTrue(app.buttons["message-action-edit"].waitForExistence(timeout: 5))
        app.buttons["message-action-edit"].tap()
        let editor = app.textFields["edit-message-input"]
        XCTAssertTrue(editor.waitForExistence(timeout: 5))
        XCTAssertEqual(editor.value as? String, original)
        app.buttons["save-message-edit"].tap()
        XCTAssertTrue(link.waitForExistence(timeout: 5))
        link.tap()
        XCTAssertTrue(app.wait(for: .runningBackground, timeout: 5))
        let safari = XCUIApplication(bundleIdentifier: "com.apple.mobilesafari")
        XCTAssertTrue(safari.wait(for: .runningForeground, timeout: 5))
    }

    @MainActor
    func testReplyCanReceiveAStampInItsThread() {
        let app = launchWorkspace()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let welcome = app.staticTexts["message-content-22222222-2222-4222-8222-222222222222"]
        openActions(for: welcome, in: app)
        app.buttons["message-action-reply"].tap()
        let input = composer(app, reply: true)
        app.typeText("スタンプ用の返信")
        app.buttons["send-reply"].tap()
        let reply = app.staticTexts["スタンプ用の返信"]
        XCTAssertTrue(reply.waitForExistence(timeout: 5))
        let prefix = "message-content-"
        XCTAssertTrue(reply.identifier.hasPrefix(prefix))
        let messageID = reply.identifier.dropFirst(prefix.count)
        openActions(for: reply, in: app)
        app.buttons["message-action-more-reactions"].tap()
        let celebrate = app.buttons["pick-reaction-🎉"]
        XCTAssertTrue(celebrate.waitForExistence(timeout: 5))
        celebrate.tap()
        let chip = app.buttons["reaction-\(messageID)-🎉"]
        XCTAssertTrue(chip.waitForExistence(timeout: 5))
        XCTAssertEqual(chip.value as? String, "1人、自分も付けています")
        capture(app, name: "返信に付けたスタンプ")
        app.typeText("次の")
        XCTAssertEqual(input.value as? String, "次の")
        openActions(for: reply, in: app)
        app.buttons["message-action-reply"].tap()
        app.typeText("返信の下書き")
        XCTAssertEqual(input.value as? String, "次の返信の下書き")
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.005, dy: 0.35))
        let end = app.coordinate(withNormalizedOffset: CGVector(dx: 0.85, dy: 0.35))
        start.press(forDuration: 0, thenDragTo: end)
        XCTAssertTrue(app.navigationBars["一般"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.navigationBars["スレッド"].exists)
        XCTAssertEqual(composer(app).value as? String, "#一般 へのメッセージ")
        openActions(for: welcome, in: app)
        app.buttons["message-action-reply"].tap()
        XCTAssertTrue(chip.waitForExistence(timeout: 5))
        XCTAssertEqual(composer(app, reply: true).value as? String, "次の返信の下書き")
    }

    @MainActor
    func testLongHistoryInitiallyShowsLatestMessage() {
        let app = launchWorkspace(longHistory: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let latest = app.staticTexts["履歴 60"]
        XCTAssertTrue(latest.waitForExistence(timeout: 10))
        XCTAssertTrue(latest.isHittable)
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.35))
        let end = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.8))
        // カードの押下表示が出てから動かしても、長押しメニューを開く前ならスクロールできる。
        start.press(forDuration: 0.25, thenDragTo: end)
        XCTAssertFalse(latest.isHittable)
        XCTAssertFalse(app.buttons["message-action-reply"].exists)
    }

    @MainActor
    func testEditOwnMessage() {
        let app = launchWorkspace()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let message = app.staticTexts["message-content-66666666-6666-4666-8666-666666666666"]
        openActions(for: message, in: app)
        XCTAssertTrue(app.buttons["message-action-delete"].exists)
        app.buttons["message-action-edit"].tap()
        let field = app.textFields["edit-message-input"]
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        field.tap()
        field.typeText("（編集確認）")
        app.buttons["save-message-edit"].tap()
        XCTAssertTrue(message.waitForExistence(timeout: 5))
        XCTAssertTrue(message.label.contains("編集確認"))
        XCTAssertTrue(app.staticTexts["編集済み"].exists)
    }

    @MainActor
    func testLongPressDeletionRequiresConfirmation() {
        let app = launchWorkspace()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let message = app.staticTexts["message-content-66666666-6666-4666-8666-666666666666"]
        openActions(for: message, in: app)
        let remove = app.buttons["message-action-delete"]
        remove.tap()
        let dialog = app.alerts["メッセージを削除しますか？"]
        let confirm = dialog.buttons["削除"]
        XCTAssertTrue(confirm.waitForExistence(timeout: 5))
        dialog.buttons["キャンセル"].tap()
        XCTAssertTrue(remove.waitForExistence(timeout: 5))
        remove.tap()
        confirm.tap()
        XCTAssertTrue(message.waitForNonExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["message-content-22222222-2222-4222-8222-222222222222"].exists)
    }

    @MainActor
    func testFailedSendKeepsDraftAndCanRetry() {
        let app = launchWorkspace(failingSend: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let field = composer(app)
        field.tap()
        field.typeText("再送する本文")
        app.buttons["send-message"].tap()
        XCTAssertTrue(app.staticTexts["send-error"].waitForExistence(timeout: 5))
        XCTAssertEqual(field.value as? String, "再送する本文")
        app.buttons["send-message"].tap()
        XCTAssertTrue(app.staticTexts["再送する本文"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["send-error"].exists)
    }

    @MainActor
    func testEncryptedConversationCannotSendPlaintext() {
        let app = launchWorkspace()
        app.buttons["channel-77777777-7777-4777-8777-777777777777"].tap()
        XCTAssertTrue(app.staticTexts["暗号化された会話"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["send-message"].exists)
    }

    @MainActor
    func testCreateChannelAndLogout() {
        let app = launchWorkspace()
        app.buttons["new-conversation"].tap()
        app.buttons["チャンネルを作成"].tap()
        let field = app.textFields["channel-name"]
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        field.tap()
        field.typeText("新しい会話")
        app.buttons["create-channel"].tap()
        XCTAssertTrue(app.navigationBars["新しい会話"].waitForExistence(timeout: 5))
        selectTab("プロフィール", in: app)
        app.buttons["workspace-settings"].tap()
        app.buttons["logout"].tap()
        let confirmation = app.sheets.buttons["ログアウト"]
        XCTAssertTrue(confirmation.waitForExistence(timeout: 5))
        confirmation.tap()
        XCTAssertTrue(app.buttons["login"].waitForExistence(timeout: 5))
    }

    @MainActor
    func testTabsPreserveThreadAndDirectMessageDrafts() {
        let app = launchWorkspace()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        openActions(for: app.staticTexts["message-content-22222222-2222-4222-8222-222222222222"], in: app)
        app.buttons["message-action-reply"].tap()
        XCTAssertTrue(app.navigationBars["スレッド"].waitForExistence(timeout: 5))
        app.typeText("スレッドの下書き")

        selectTab("DM", in: app, dismissingKeyboard: true)
        app.buttons["new-dm"].tap()
        app.buttons["dm-recipient-44444444-4444-4444-8444-444444444444"].tap()
        app.buttons["start-dm"].tap()
        XCTAssertTrue(app.navigationBars["さくら"].waitForExistence(timeout: 5))
        let dm = composer(app)
        dm.tap()
        dm.typeText("DMの下書き")

        selectTab("チャンネル", in: app, dismissingKeyboard: true)
        XCTAssertTrue(app.navigationBars["スレッド"].waitForExistence(timeout: 5))
        XCTAssertEqual(composer(app, reply: true).value as? String, "スレッドの下書き")
        selectTab("DM", in: app)
        XCTAssertTrue(app.navigationBars["さくら"].waitForExistence(timeout: 5))
        XCTAssertEqual(composer(app).value as? String, "DMの下書き")
    }

    @MainActor
    func testJoinedThreadOpensFromChannelDirectory() {
        let app = launchWorkspace(threadDirectory: true)
        let entry = app.buttons["channel-thread-22222222-2222-4222-8222-222222222222"]
        XCTAssertTrue(entry.waitForExistence(timeout: 5))
        entry.tap()
        let thread = app.navigationBars["スレッド"]
        XCTAssertTrue(thread.waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["参加中のスレッドです"].waitForExistence(timeout: 5))
        thread.buttons.element(boundBy: 0).tap()
        let channel = app.navigationBars["一般"]
        XCTAssertTrue(channel.waitForExistence(timeout: 5))
        channel.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(app.navigationBars["チャンネル"].waitForExistence(timeout: 5))
        XCTAssertTrue(entry.exists)
    }

    @MainActor
    func testJoinChannelAndOpenNotification() {
        let app = launchWorkspace()
        let joined = app.buttons["channel-aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"]
        XCTAssertFalse(joined.exists)
        app.buttons["new-conversation"].tap()
        app.buttons["チャンネルを探す"].tap()
        app.buttons["browse-channel-aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"].tap()
        let bar = app.navigationBars["雑談"]
        XCTAssertTrue(bar.waitForExistence(timeout: 5))
        bar.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(joined.waitForExistence(timeout: 5))

        selectTab("通知", in: app)
        let mention = app.buttons["inbox-open-22222222-2222-4222-8222-222222222222"]
        XCTAssertTrue(mention.waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["承認"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["inbox-read-all"].waitForNonExistence(timeout: 5))
        mention.tap()
        XCTAssertTrue(app.navigationBars["一般"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["message-content-22222222-2222-4222-8222-222222222222"].exists)
        XCTAssertTrue(app.tabBars.buttons["チャンネル"].isSelected)
    }

    @MainActor
    func testEditPublicProfile() {
        let app = launchWorkspace()
        selectTab("プロフィール", in: app)
        let edit = app.buttons["profile-edit"]
        XCTAssertTrue(edit.waitForExistence(timeout: 5))
        edit.tap()
        let name = app.textFields["profile-name"]
        name.tap()
        name.typeText("動物園の花子")
        let textView = app.textViews["profile-summary"]
        let summary = textView.exists ? textView : app.textFields["profile-summary"]
        summary.tap()
        summary.typeText("よろしくお願いします")
        app.buttons["profile-save"].tap()
        XCTAssertTrue(app.staticTexts["よろしくお願いします"].waitForExistence(timeout: 5))
        selectTab("通知", in: app)
        selectTab("プロフィール", in: app)
        edit.tap()
        XCTAssertEqual(name.value as? String, "動物園の花子")
        XCTAssertEqual(summary.value as? String, "よろしくお願いします")
    }
}
