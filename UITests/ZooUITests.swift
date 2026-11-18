import XCTest

final class ZooUITests: XCTestCase {
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
                                 threadDirectory: Bool = false) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace"]
        if failingSend { app.launchArguments.append("--uitesting-send-failure") }
        if liveMessage { app.launchArguments.append("--uitesting-live-message") }
        if longHistory { app.launchArguments.append("--uitesting-long-history") }
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
        XCTAssertEqual(field.value as? String, "メッセージを入力")
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
        XCTAssertEqual(composer(app).value as? String, "メッセージを入力")
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
