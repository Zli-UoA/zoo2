import AVKit
import XCTest

final class ZooUITests: XCTestCase {
    @MainActor
    func testPDFAttachmentPreview() {
        defer { XCUIDevice.shared.orientation = .portrait }
        let app = launchWorkspace(pdf: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let open = app.buttons["PDFを全画面で表示：確認資料.pdf"]
        XCTAssertTrue(open.waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["PDFを表示"].exists)
        XCTAssertGreaterThan(open.frame.height, 150)
        capture(app, name: "会話内のPDFプレビュー")
        open.tap()
        let pdf = app.descendants(matching: .any)["attachment-pdf-view"].firstMatch
        XCTAssertTrue(pdf.waitForExistence(timeout: 5))
        XCTAssertTrue(app.navigationBars["確認資料.pdf"].exists)
        XCTAssertEqual(app.staticTexts["pdf-page-count"].label, "2ページ")
        XCTAssertLessThan(app.navigationBars["確認資料.pdf"].frame.minY, 100)
        XCTAssertGreaterThan(pdf.frame.height, app.frame.height * 0.7)
        capture(app, name: "添付PDFの全画面表示")
        rotateToLandscape(app)
        capture(app, name: "添付PDFだけを横向きで表示")
        pdf.pinch(withScale: 1.5, velocity: 1)
        pdf.swipeUp()
        capture(app, name: "添付PDFの拡大とページ移動")
        app.buttons["閉じる"].tap()
        XCTAssertTrue(open.waitForExistence(timeout: 5))
        waitForConversationOrientation(app)
    }

    @MainActor
    func testSensitivePDFCanRetryAfterFailure() {
        let app = launchWorkspace(pdf: true, pdfFailure: true, sensitivePDF: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let reveal = app.buttons["センシティブな添付を表示"]
        XCTAssertTrue(reveal.waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["PDFを全画面で表示：確認資料.pdf"].exists)
        reveal.tap()
        let open = app.buttons["PDFを全画面で表示：確認資料.pdf"]
        XCTAssertTrue(open.waitForExistence(timeout: 5))
        open.tap()
        XCTAssertTrue(app.staticTexts["添付を開けません"].waitForExistence(timeout: 5))
        app.buttons["再読み込み"].tap()
        XCTAssertTrue(app.descendants(matching: .any)["attachment-pdf-view"].firstMatch.waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["添付を開けません"].exists)
        app.buttons["閉じる"].tap()
        XCTAssertTrue(open.waitForExistence(timeout: 5))
    }

    @MainActor
    func testFileAttachmentsCanBeRemovedAndRetried() {
        let app = launchWorkspace(failingSend: true, attachments: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let remove = app.buttons["添付を削除：添付確認.txt"]
        XCTAssertTrue(remove.waitForExistence(timeout: 5))
        XCTAssertTrue(remove.isHittable)
        XCTAssertTrue(app.staticTexts["こんにちは！ Zoo の iOS 版へようこそ。"].isHittable)
        capture(app, name: "ファイルを添付した入力欄")
        remove.tap()
        XCTAssertFalse(remove.exists)
        let remaining = app.buttons["添付を削除：削除確認.txt"]
        XCTAssertTrue(remaining.isHittable)
        let sensitive = app.descendants(matching: .any)["draft-sensitive-削除確認.txt"].firstMatch
        XCTAssertEqual(sensitive.value as? String, "オフ")
        sensitive.tap()
        XCTAssertEqual(sensitive.value as? String, "オン")
        let send = app.buttons["send-message"]
        XCTAssertTrue(send.isEnabled)
        send.tap()
        XCTAssertTrue(app.staticTexts["send-error"].waitForExistence(timeout: 5))
        XCTAssertTrue(remaining.exists)
        XCTAssertEqual(sensitive.value as? String, "オン")
        send.tap()
        XCTAssertTrue(remaining.waitForNonExistence(timeout: 5))
        let reveal = app.buttons["センシティブな添付を表示"]
        XCTAssertTrue(reveal.waitForExistence(timeout: 5))
        reveal.tap()
        XCTAssertTrue(app.staticTexts["削除確認.txt"].waitForExistence(timeout: 5))
        XCTAssertFalse(send.isEnabled)
        capture(app, name: "本文なしの添付投稿を再送した会話")
    }

    @MainActor
    func testFilePickerCancellationPreservesDraft() {
        let app = launchWorkspace()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let field = composer(app)
        field.tap()
        field.typeText("添付を選ぶ前の下書き")
        app.buttons["composer-additions"].tap()
        let attach = app.buttons["composer-attach-file"]
        XCTAssertTrue(attach.waitForExistence(timeout: 5))
        capture(app, name: "入力中に開いた添付メニュー")
        attach.tap()
        let cancel = app.buttons.matching(NSPredicate(format: "label IN %@", ["Cancel", "キャンセル"])).firstMatch
        XCTAssertTrue(cancel.waitForExistence(timeout: 15))
        cancel.tap()
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        XCTAssertEqual(field.value as? String, "添付を選ぶ前の下書き")
        XCTAssertTrue(app.buttons["send-message"].isEnabled)
        XCTAssertFalse(app.staticTexts["send-error"].exists)
    }

    @MainActor
    func testPhotoPickerCancellationPreservesDraft() {
        let app = launchWorkspace(attachments: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let field = composer(app)
        field.tap()
        field.typeText("写真を選ぶ前の下書き")
        app.buttons["composer-additions"].tap()
        let photos = app.buttons["composer-attach-photos"]
        XCTAssertTrue(photos.waitForExistence(timeout: 5))
        photos.tap()
        let cancel = app.buttons.matching(NSPredicate(format: "label IN %@", ["Cancel", "キャンセル", "Close", "閉じる"])).firstMatch
        XCTAssertTrue(cancel.waitForExistence(timeout: 5))
        cancel.tap()
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        XCTAssertEqual(field.value as? String, "写真を選ぶ前の下書き")
        XCTAssertTrue(app.buttons["添付を削除：添付確認.txt"].exists)
        XCTAssertTrue(app.buttons["send-message"].isEnabled)
        XCTAssertFalse(app.staticTexts["send-error"].exists)
    }

    @MainActor
    func testActiveCallIndicatorInPinnedAndRegularChannels() {
        let app = launchWorkspace(chatDetails: true, activeCalls: true)
        let channel = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        let indicator = app.images["channel-active-call-11111111-1111-4111-8111-111111111111"]
        XCTAssertTrue(indicator.waitForExistence(timeout: 5))
        XCTAssertEqual(indicator.label, "通話中")
        XCTAssertGreaterThan(indicator.frame.minX, channel.frame.midX)
        XCTAssertTrue(app.buttons["channel-section-pinned"].exists)
        XCTAssertFalse(app.buttons["active-call"].exists)
        XCTAssertFalse(app.buttons["channel-55555555-5555-4555-8555-555555555555"].label.contains("通話中"))
        capture(app, name: "ピン留めしたチャンネルの通話中表示")
        channel.press(forDuration: 0.8)
        app.buttons["toggle-channel-pin"].tap()
        XCTAssertTrue(app.buttons["channel-section-pinned"].waitForNonExistence(timeout: 5))
        XCTAssertTrue(channel.label.contains("通話中"))
        XCTAssertTrue(channel.label.contains("未読"))
        capture(app, name: "通常一覧の通話中表示と未読マーク")
    }

    @MainActor
    func testActiveCallIndicatorFollowsLocalCall() {
        let app = launchWorkspace(call: true)
        let channel = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        XCTAssertFalse(channel.label.contains("通話中"))
        channel.tap()
        app.buttons["start-call"].tap()
        XCTAssertTrue(app.buttons["call-mute"].waitForExistence(timeout: 5))
        app.buttons["閉じる"].tap()
        app.navigationBars.buttons.firstMatch.tap()
        XCTAssertTrue(channel.waitForExistence(timeout: 5))
        XCTAssertTrue(channel.label.contains("通話中"))
        app.buttons["active-call"].tap()
        app.buttons["call-end"].tap()
        XCTAssertTrue(channel.waitForExistence(timeout: 5))
        XCTAssertFalse(channel.label.contains("通話中"))
    }

    @MainActor
    func testChannelCallInvitesAndCancellation() {
        let app = launchWorkspace(call: true, callInviteFailure: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        app.buttons["start-call"].tap()
        let invite = app.buttons["call-invite"]
        XCTAssertTrue(invite.waitForExistence(timeout: 5))
        XCTAssertTrue(invite.wait(for: \.isEnabled, toEqual: true, timeout: 5))
        invite.tap()
        let member = app.buttons["call-invite-member-44444444-4444-4444-8444-444444444444"]
        XCTAssertTrue(member.waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["call-invite-member-33333333-3333-4333-8333-333333333333"].exists)
        let send = app.buttons["call-send-invitations"]
        XCTAssertFalse(send.isEnabled)
        member.tap()
        capture(app, name: "チャンネル通話の招待メンバー選択")
        send.tap()
        XCTAssertTrue(app.staticTexts["call-invite-error"].waitForExistence(timeout: 5))
        XCTAssertEqual(member.value as? String, "選択済み")
        XCTAssertTrue(send.isEnabled)
        send.tap()
        let cancel = app.buttons["call-cancel-44444444-4444-4444-8444-444444444444"]
        XCTAssertTrue(cancel.waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["call-mute"].isEnabled)
        capture(app, name: "通話への招待を送信した画面")
        invite.tap()
        XCTAssertTrue(app.buttons["call-send-invitations"].waitForExistence(timeout: 5))
        XCTAssertFalse(member.exists)
        app.buttons["call-invite-close"].tap()
        XCTAssertTrue(cancel.waitForExistence(timeout: 5))
        cancel.tap()
        XCTAssertTrue(cancel.waitForNonExistence(timeout: 5))
        app.buttons["call-end"].tap()
    }

    @MainActor
    func testDMCallHasNoManualInvitation() {
        let app = launchWorkspace(call: true)
        selectTab("DM", in: app)
        app.buttons["new-dm"].tap()
        app.buttons["dm-recipient-44444444-4444-4444-8444-444444444444"].tap()
        app.buttons["start-dm"].tap()
        let start = app.buttons["start-call"]
        XCTAssertTrue(start.waitForExistence(timeout: 5))
        start.tap()
        XCTAssertTrue(app.buttons["call-mute"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["call-invite"].exists)
        app.buttons["call-end"].tap()
    }

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
    func testChannelSectionsCollapseIndependentlyWithThreads() {
        let app = launchWorkspace(threadDirectory: true, chatDetails: true)
        let pinned = app.buttons["channel-section-pinned"]
        let list = app.buttons["channel-section-list"]
        let general = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        let development = app.buttons["channel-55555555-5555-4555-8555-555555555555"]
        let thread = app.buttons["channel-thread-22222222-2222-4222-8222-222222222222"]
        XCTAssertTrue(thread.waitForExistence(timeout: 5))

        pinned.tap()
        XCTAssertTrue(general.waitForNonExistence(timeout: 5))
        XCTAssertFalse(thread.exists)
        XCTAssertTrue(development.exists)
        XCTAssertEqual(pinned.value as? String, "折り畳み中")

        list.tap()
        XCTAssertTrue(development.waitForNonExistence(timeout: 5))
        XCTAssertTrue(pinned.isHittable)
        XCTAssertTrue(list.isHittable)
        capture(app, name: "すべてのセクションを折り畳んだ一覧")

        list.tap()
        XCTAssertTrue(development.waitForExistence(timeout: 5))
        development.tap()
        let bar = app.navigationBars["開発"]
        XCTAssertTrue(bar.waitForExistence(timeout: 5))
        bar.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(pinned.waitForExistence(timeout: 5))
        XCTAssertFalse(general.exists)
        XCTAssertEqual(pinned.value as? String, "折り畳み中")

        selectTab("DM", in: app)
        selectTab("チャンネル", in: app)
        XCTAssertEqual(pinned.value as? String, "折り畳み中")
        pinned.tap()
        XCTAssertTrue(general.waitForExistence(timeout: 5))
        XCTAssertTrue(thread.waitForExistence(timeout: 5))
        capture(app, name: "セクションを再び展開した一覧")
    }

    @MainActor
    func testChannelSearchShowsCollapsedSectionsAndRestoresState() {
        let app = launchWorkspace(chatDetails: true)
        let searchButton = app.buttons["search-conversation"]
        let search = app.textFields["conversation-search-input"]
        let additions = app.buttons["new-conversation"]
        XCTAssertTrue(searchButton.waitForExistence(timeout: 5))
        XCTAssertLessThan(searchButton.frame.midX, additions.frame.midX)
        XCTAssertFalse(search.exists)
        capture(app, name: "検索バーを省き追加ボタンの左に検索を配置")
        let pinned = app.buttons["channel-section-pinned"]
        let list = app.buttons["channel-section-list"]
        let general = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        let development = app.buttons["channel-55555555-5555-4555-8555-555555555555"]
        pinned.tap()
        list.tap()
        XCTAssertTrue(general.waitForNonExistence(timeout: 5))
        XCTAssertFalse(development.exists)

        searchButton.tap()
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        search.typeText("一般")
        XCTAssertTrue(general.waitForExistence(timeout: 5))
        XCTAssertFalse(development.exists)
        XCTAssertEqual(pinned.value as? String, "展開中")
        XCTAssertFalse(pinned.isEnabled)
        capture(app, name: "Liquid Glassの検索欄と絞り込み結果")

        app.buttons["clear-conversation-search"].tap()
        XCTAssertTrue(general.waitForNonExistence(timeout: 5))
        XCTAssertTrue(search.exists)
        XCTAssertTrue(app.keyboards.firstMatch.exists)
        search.typeText("一般")
        XCTAssertTrue(general.waitForExistence(timeout: 5))

        let closeSearch = app.buttons["close-conversation-search"]
        XCTAssertTrue(closeSearch.waitForExistence(timeout: 5))
        XCTAssertEqual(closeSearch.label, "検索を閉じる")
        closeSearch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.05)).tap()
        XCTAssertTrue(search.waitForNonExistence(timeout: 5))
        XCTAssertTrue(searchButton.waitForExistence(timeout: 5))
        XCTAssertTrue(general.waitForNonExistence(timeout: 5))
        XCTAssertFalse(development.exists)
        XCTAssertEqual(pinned.value as? String, "折り畳み中")
        XCTAssertEqual(list.value as? String, "折り畳み中")
        XCTAssertTrue(pinned.isEnabled)
        pinned.tap()
        XCTAssertTrue(general.waitForExistence(timeout: 5))
    }

    @MainActor
    func testChannelSearchSupportsAccessibilityTextSize() {
        let app = launchWorkspace(chatDetails: true, largeText: true)
        app.buttons["search-conversation"].tap()
        let search = app.textFields["conversation-search-input"]
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        search.typeText("一般")
        XCTAssertGreaterThan(search.frame.width, app.frame.width / 2)
        XCTAssertTrue(search.isHittable)
        let closeSearch = app.buttons["close-conversation-search"]
        XCTAssertTrue(closeSearch.isHittable)
        XCTAssertLessThanOrEqual(closeSearch.frame.maxY, app.keyboards.firstMatch.frame.minY)
        capture(app, name: "文字を拡大したチャンネル検索")
        closeSearch.tap()
        XCTAssertTrue(search.waitForNonExistence(timeout: 5))
        app.buttons["search-conversation"].tap()
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        closeSearch.tap()
        XCTAssertTrue(search.waitForNonExistence(timeout: 5))
    }

    @MainActor
    func testDirectMessageSearchUsesSharedDirectorySearch() {
        let app = launchWorkspace()
        selectTab("DM", in: app)
        let searchButton = app.buttons["search-conversation"]
        let search = app.textFields["conversation-search-input"]
        let create = app.buttons["new-dm"]
        XCTAssertTrue(searchButton.waitForExistence(timeout: 5))
        XCTAssertLessThan(searchButton.frame.midX, create.frame.midX)
        XCTAssertFalse(search.exists)

        for (id, name) in [("44444444-4444-4444-8444-444444444444", "さくら"),
                           ("99999999-9999-4999-8999-999999999999", "あおい")] {
            create.tap()
            app.buttons["dm-recipient-\(id)"].tap()
            app.buttons["start-dm"].tap()
            let bar = app.navigationBars[name]
            XCTAssertTrue(bar.waitForExistence(timeout: 5))
            bar.buttons.element(boundBy: 0).tap()
            XCTAssertTrue(searchButton.waitForExistence(timeout: 5))
        }

        let sakura = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@ AND label CONTAINS %@", "channel-", "さくら")).firstMatch
        let aoi = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@ AND label CONTAINS %@", "channel-", "あおい")).firstMatch
        XCTAssertTrue(sakura.waitForExistence(timeout: 5))
        XCTAssertTrue(aoi.exists)
        capture(app, name: "DM作成ボタンの左に検索を配置")

        searchButton.tap()
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        XCTAssertEqual(search.placeholderValue, "DMを検索")
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        search.typeText("さくら")
        XCTAssertTrue(sakura.waitForExistence(timeout: 5))
        XCTAssertTrue(aoi.waitForNonExistence(timeout: 5))
        capture(app, name: "DMも共通のLiquid Glass検索欄で絞り込み")

        let clear = app.buttons["clear-conversation-search"]
        clear.tap()
        XCTAssertTrue(aoi.waitForExistence(timeout: 5))
        search.typeText("該当者なし")
        XCTAssertTrue(sakura.waitForNonExistence(timeout: 5))
        XCTAssertFalse(aoi.exists)
        clear.tap()
        search.typeText("さくら")
        app.buttons["close-conversation-search"].tap()
        XCTAssertTrue(search.waitForNonExistence(timeout: 5))
        XCTAssertTrue(sakura.waitForExistence(timeout: 5))
        XCTAssertTrue(aoi.exists)
        selectTab("チャンネル", in: app)
        XCTAssertFalse(search.exists)
        XCTAssertTrue(searchButton.waitForExistence(timeout: 5))
    }

    @MainActor
    func testSharedSearchInDiscoveryAndRecipientPicker() {
        let app = launchWorkspace(dark: true)
        app.buttons["new-conversation"].tap()
        app.buttons["チャンネルを探す"].tap()
        let channel = app.buttons["browse-channel-aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"]
        XCTAssertTrue(channel.waitForExistence(timeout: 5))
        let discovery = app.textFields["channel-browser-search-input"]
        XCTAssertFalse(discovery.exists)
        app.buttons["search-channel-browser"].tap()
        XCTAssertTrue(discovery.waitForExistence(timeout: 5))
        discovery.typeText("該当なし")
        XCTAssertTrue(channel.waitForNonExistence(timeout: 5))
        app.buttons["clear-channel-browser-search"].tap()
        discovery.typeText("雑談")
        XCTAssertTrue(channel.waitForExistence(timeout: 5))
        capture(app, name: "ダーク表示のチャンネル検索")
        app.buttons["close-channel-browser-search"].tap()
        XCTAssertTrue(discovery.waitForNonExistence(timeout: 5))
        app.buttons["閉じる"].tap()

        selectTab("DM", in: app)
        capture(app, name: "チャンネルと揃えたDM一覧")
        app.buttons["new-dm"].tap()
        let sakura = app.buttons["dm-recipient-44444444-4444-4444-8444-444444444444"]
        let aoi = app.buttons["dm-recipient-99999999-9999-4999-8999-999999999999"]
        XCTAssertTrue(sakura.waitForExistence(timeout: 5))
        app.buttons["search-people"].tap()
        let people = app.textFields["people-search-input"]
        XCTAssertTrue(people.waitForExistence(timeout: 5))
        people.typeText("さくら")
        XCTAssertTrue(aoi.waitForNonExistence(timeout: 5))
        sakura.tap()
        app.buttons["clear-people-search"].tap()
        people.typeText("あおい")
        XCTAssertTrue(aoi.waitForExistence(timeout: 5))
        XCTAssertTrue(sakura.exists)
        XCTAssertTrue(sakura.isSelected)
        capture(app, name: "共通検索と選択状態を保つDMの宛先一覧")
        app.buttons["close-people-search"].tap()
        XCTAssertTrue(people.waitForNonExistence(timeout: 5))
        XCTAssertTrue(sakura.isSelected)
        app.buttons["閉じる"].tap()

        selectTab("プロフィール", in: app)
        XCTAssertTrue(app.buttons["profile-edit"].waitForExistence(timeout: 5))
        capture(app, name: "ダーク表示のプロフィール")
        app.buttons["workspace-settings"].tap()
        XCTAssertTrue(app.navigationBars["設定"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["logout"].exists)
        capture(app, name: "既存の表示を維持した設定画面")
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
        // 操作対象ではないエラー表示は、スクロール後のヒット判定ではなく表示範囲で確認する。
        XCTAssertTrue(error.exists)
        XCTAssertGreaterThan(error.frame.height, 0)
        XCTAssertTrue(app.frame.contains(error.frame))
        XCTAssertGreaterThanOrEqual(error.frame.minY, app.navigationBars.firstMatch.frame.maxY - 1)
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
    func testReactionsWrapAndRemainInteractive() {
        let app = launchWorkspace(wrappedReactions: true)
        let messageID = "22222222-2222-4222-8222-222222222222"
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let celebration = app.buttons["reaction-\(messageID)-🎉"]
        XCTAssertTrue(celebration.waitForExistence(timeout: 10))
        let chips = app.buttons.matching(NSPredicate(format: "identifier BEGINSWITH %@", "reaction-\(messageID)-")).allElementsBoundByIndex
        XCTAssertEqual(chips.count, 12)
        let chipSize = celebration.frame.size
        XCTAssertGreaterThan((chips.map { $0.frame.minY }.max() ?? 0) - (chips.map { $0.frame.minY }.min() ?? 0), 40)
        for chip in chips {
            XCTAssertEqual(chip.frame.width, chipSize.width, accuracy: 1)
            XCTAssertEqual(chip.frame.height, chipSize.height, accuracy: 1)
            XCTAssertTrue(chip.isHittable)
            XCTAssertGreaterThanOrEqual(chip.frame.minX, app.frame.minX)
            XCTAssertLessThanOrEqual(chip.frame.maxX, app.frame.maxX)
        }
        capture(app, name: "複数行に折り返すスタンプ")
        celebration.tap()
        let selected = XCTNSPredicateExpectation(predicate: NSPredicate(format: "value == %@", "2人、自分も付けています"), object: celebration)
        XCTAssertEqual(XCTWaiter.wait(for: [selected], timeout: 5), .completed)
        celebration.press(forDuration: 0.8)
        XCTAssertTrue(app.staticTexts["2人がリアクションしました"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["message-action-more-reactions"].exists)
    }

    @MainActor
    func testMessageCardWhitespaceOpensActions() {
        let app = launchWorkspace()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let message = app.staticTexts["message-content-22222222-2222-4222-8222-222222222222"]
        let reaction = app.buttons["reaction-22222222-2222-4222-8222-222222222222-👍"]
        XCTAssertTrue(message.waitForExistence(timeout: 5))
        XCTAssertTrue(reaction.waitForExistence(timeout: 5))
        let author = app.staticTexts["さくら"].firstMatch
        let points = [
            CGPoint(x: app.frame.minX + 5, y: message.frame.midY),
            CGPoint(x: app.frame.maxX - 5, y: message.frame.midY),
            CGPoint(x: message.frame.minX - 6, y: message.frame.midY),
            CGPoint(x: message.frame.midX, y: author.frame.minY - 4),
            CGPoint(x: message.frame.midX, y: message.frame.minY - 3),
            CGPoint(x: message.frame.midX, y: reaction.frame.maxY + 4)
        ]
        for point in points {
            app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: point.x - app.frame.minX, dy: point.y - app.frame.minY)).press(forDuration: 0.8)
            XCTAssertTrue(app.buttons["message-action-more-reactions"].waitForExistence(timeout: 5), "余白の長押し位置：\(point)")
            XCTAssertTrue(app.buttons["message-action-reply"].exists)
            XCTAssertFalse(app.buttons["message-action-edit"].exists)
            app.buttons["message-action-copy"].tap()
            XCTAssertTrue(app.buttons["message-action-copy"].waitForNonExistence(timeout: 5))
        }
    }

    @MainActor
    func testImageWhitespaceOpensActionsWithoutInterceptingImage() {
        let app = launchWorkspace(tallImage: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let image = app.buttons["画像を拡大：縦長画像.png"]
        XCTAssertTrue(image.waitForExistence(timeout: 5))
        for x in [app.frame.minX + 5, (image.frame.maxX + app.frame.maxX) / 2] {
            app.coordinate(withNormalizedOffset: .zero)
                .withOffset(CGVector(dx: x - app.frame.minX, dy: image.frame.midY - app.frame.minY)).press(forDuration: 0.8)
            XCTAssertTrue(app.buttons["message-action-more-reactions"].waitForExistence(timeout: 5))
            XCTAssertTrue(app.buttons["message-action-reply"].exists)
            XCTAssertFalse(app.buttons["attachment-save"].exists)
            app.buttons["message-action-copy"].tap()
            XCTAssertTrue(app.buttons["message-action-copy"].waitForNonExistence(timeout: 5))
        }
        image.tap()
        XCTAssertTrue(app.navigationBars["縦長画像.png"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["message-action-more-reactions"].exists)
        app.buttons["閉じる"].tap()
        XCTAssertTrue(image.waitForExistence(timeout: 5))
        image.press(forDuration: 0.8)
        XCTAssertTrue(app.buttons["attachment-save"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["message-action-more-reactions"].exists)
    }

    @MainActor
    func testImageGalleryWhitespaceOpensActions() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-multiple-images"]
        app.launch()
        let channel = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        XCTAssertTrue(channel.waitForExistence(timeout: 10))
        channel.tap()
        let image = app.buttons["画像を拡大：さくら.png"]
        XCTAssertTrue(image.waitForExistence(timeout: 5))
        for _ in 0..<4 where !image.isHittable { app.scrollViews["message-list"].swipeDown() }
        for x in [image.frame.minX - 8, image.frame.maxX + 4] {
            app.coordinate(withNormalizedOffset: .zero)
                .withOffset(CGVector(dx: x - app.frame.minX, dy: image.frame.midY - app.frame.minY)).press(forDuration: 0.8)
            XCTAssertTrue(app.buttons["message-action-more-reactions"].waitForExistence(timeout: 5))
            XCTAssertTrue(app.buttons["message-action-reply"].exists)
            XCTAssertFalse(app.buttons["attachment-save"].exists)
            app.scrollViews.containing(.button, identifier: "message-action-reply").firstMatch.swipeDown()
            XCTAssertTrue(app.buttons["message-action-more-reactions"].waitForNonExistence(timeout: 5))
        }
        let gallery = app.scrollViews["message-images-22222222-2222-4222-8222-222222222222"]
        gallery.swipeLeft()
        XCTAssertFalse(app.buttons["message-action-more-reactions"].exists)
        gallery.swipeRight()
        image.tap()
        XCTAssertTrue(app.navigationBars["さくら.png"].waitForExistence(timeout: 5))
        app.buttons["閉じる"].tap()
        XCTAssertTrue(image.waitForExistence(timeout: 5))
        image.press(forDuration: 0.8)
        XCTAssertTrue(app.buttons["attachment-save"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["message-action-more-reactions"].exists)
    }

    @MainActor
    func testShortImageGalleryTrailingWhitespaceOpensActions() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-multiple-images", "--uitesting-short-image-gallery"]
        app.launch()
        let channel = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        XCTAssertTrue(channel.waitForExistence(timeout: 10))
        channel.tap()
        let image = app.buttons["画像を拡大：縦長画像.png"]
        XCTAssertTrue(image.waitForExistence(timeout: 5))
        XCTAssertLessThan(image.frame.maxX, app.frame.maxX - 20)
        app.coordinate(withNormalizedOffset: .zero)
            .withOffset(CGVector(dx: (image.frame.maxX + app.frame.maxX) / 2 - app.frame.minX, dy: image.frame.midY - app.frame.minY))
            .press(forDuration: 0.8)
        XCTAssertTrue(app.buttons["message-action-more-reactions"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["message-action-reply"].exists)
        XCTAssertFalse(app.buttons["attachment-save"].exists)
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
    func testTallImageFitsZoomsAndDismisses() {
        defer { XCUIDevice.shared.orientation = .portrait }
        let app = launchWorkspace(tallImage: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let open = app.buttons["画像を拡大：縦長画像.png"]
        XCTAssertTrue(open.waitForExistence(timeout: 5))
        open.tap()
        let viewport = app.scrollViews["attachment-image-view"]
        let image = app.images["attachment-preview-image"]
        XCTAssertTrue(image.waitForExistence(timeout: 5))
        XCTAssertLessThan(app.navigationBars["縦長画像.png"].frame.minY, 100)
        XCTAssertGreaterThan(viewport.frame.height, app.frame.height * 0.7)
        let fitted = image.frame
        XCTAssertTrue(viewport.frame.insetBy(dx: -1, dy: -1).contains(fitted))
        viewport.swipeLeft()
        XCTAssertEqual(image.frame.minX, fitted.minX, accuracy: 1)
        capture(app, name: "縦長画像の全体表示")
        image.pinch(withScale: UIDevice.current.userInterfaceIdiom == .pad ? 4 : 2, velocity: 1)
        XCTAssertGreaterThan(image.frame.width, fitted.width * 1.3)
        XCTAssertTrue(app.buttons["閉じる"].isHittable)
        capture(app, name: "画像をピンチで拡大")
        image.doubleTap()
        XCTAssertEqual(image.frame.width, fitted.width, accuracy: 1)
        viewport.swipeDown()
        XCTAssertTrue(viewport.waitForNonExistence(timeout: 5))
        XCTAssertTrue(open.isHittable)
        open.tap()
        XCTAssertTrue(image.waitForExistence(timeout: 5))
        image.doubleTap()
        XCTAssertGreaterThan(image.frame.width, fitted.width * 1.3)
        rotateToLandscape(app)
        capture(app, name: "添付画像だけを横向きで表示")
        app.buttons["閉じる"].tap()
        XCTAssertTrue(viewport.waitForNonExistence(timeout: 5))
        waitForConversationOrientation(app)
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
        defer { XCUIDevice.shared.orientation = .portrait }
        let app = launchWorkspace(video: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let play = app.buttons["動画を再生：確認用.mp4"]
        XCTAssertTrue(play.waitForExistence(timeout: 10))
        capture(app, name: "動画の先頭フレームと再生ボタン")
        play.tap()
        let player = app.descendants(matching: .any)["attachment-video-player"].firstMatch
        XCTAssertTrue(player.waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["channel-details"].exists)
        if UIDevice.current.userInterfaceIdiom == .pad { rotateToLandscape(app) }
        else { assertStaysPortraitWhenDeviceRotates(app) }
        XCTAssertFalse(app.navigationBars["確認用.mp4"].exists)
        XCTAssertFalse(app.staticTexts["この動画を再生できません。再読み込みをお試しください。"].exists)
        if !app.buttons["Fullscreen Button"].isHittable {
            player.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.15)).tap()
        }
        XCTAssertTrue(app.buttons["Fullscreen Button"].waitForExistence(timeout: 5))
        capture(app, name: "チャンネル内の動画再生")
        app.buttons["Fullscreen Button"].tap()
        rotateToLandscape(app)
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.25)).tap()
        XCTAssertTrue(app.buttons["Close Button"].waitForExistence(timeout: 5))
        capture(app, name: "動画の全画面再生")
        app.buttons["Close Button"].tap()
        XCTAssertTrue(app.buttons["channel-details"].waitForExistence(timeout: 5))
        capture(app, name: "動画の全画面表示を閉じた会話")
        XCTAssertTrue(player.waitForExistence(timeout: 5))
        waitForConversationOrientation(app)
    }

    @MainActor
    func testVideoPictureInPictureRestoresPortrait() throws {
        try XCTSkipUnless(UIDevice.current.userInterfaceIdiom == .phone, "縦向き固定はiPhoneのみ")
        try XCTSkipUnless(AVPictureInPictureController.isPictureInPictureSupported(), "PiP非対応環境では実機で確認する")
        defer { XCUIDevice.shared.orientation = .portrait }
        let app = launchWorkspace(video: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let play = app.buttons["動画を再生：確認用.mp4"]
        XCTAssertTrue(play.waitForExistence(timeout: 10))
        play.tap()
        let player = app.descendants(matching: .any)["attachment-video-player"].firstMatch
        XCTAssertTrue(player.waitForExistence(timeout: 5))
        if !app.buttons["Fullscreen Button"].isHittable {
            player.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.15)).tap()
        }
        app.buttons["Fullscreen Button"].tap()
        rotateToLandscape(app)
        let pictureInPicture = app.buttons.matching(NSPredicate(
            format: "identifier CONTAINS[c] 'picture' OR identifier CONTAINS[c] 'pip' OR label CONTAINS 'ピクチャ'"
        )).firstMatch
        if !app.buttons["Close Button"].isHittable {
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.25)).tap()
        }
        XCTAssertTrue(pictureInPicture.wait(for: \.isHittable, toEqual: true, timeout: 5))
        pictureInPicture.tap()
        XCTAssertTrue(app.buttons["channel-details"].waitForExistence(timeout: 5))
        waitForPortrait(app)
        capture(app, name: "動画をPiPへ移した後は会話を縦表示に戻す")
    }

    @MainActor
    func testReadSaveRetriesWhileConversationStaysOpen() {
        let app = launchWorkspace(chatDetails: true, readSaveRetry: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let error = app.staticTexts["read-save-error"]
        XCTAssertTrue(error.waitForExistence(timeout: 10))
        XCTAssertTrue(error.waitForNonExistence(timeout: 10))
        app.navigationBars.buttons.firstMatch.tap()
        app.buttons["未読のみ"].tap()
        XCTAssertTrue(app.staticTexts["未読の会話はありません"].waitForExistence(timeout: 5))
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
    func testComposerMenuInsertsMentionAndEmoji() {
        let app = launchWorkspace(dark: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let field = composer(app)
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        capture(app, name: "Slackに合わせた通常時の入力欄")
        field.tap()
        app.buttons["composer-additions"].tap()
        let mention = app.buttons["メンションを挿入"]
        XCTAssertTrue(mention.waitForExistence(timeout: 5))
        mention.tap()
        XCTAssertTrue(app.buttons["mention-sakura"].waitForExistence(timeout: 5))
        capture(app, name: "Slackに合わせた入力欄と候補パネル")
        app.buttons["mention-sakura"].tap()
        field.typeText("ありがとう ")
        app.buttons["composer-additions"].tap()
        let emoji = app.buttons["スタンプを挿入"]
        XCTAssertTrue(emoji.waitForExistence(timeout: 5))
        emoji.tap()
        XCTAssertTrue(app.buttons["pick-reaction-🎉"].waitForExistence(timeout: 5))
        app.buttons["pick-reaction-🎉"].tap()
        XCTAssertTrue(app.buttons["send-message"].wait(for: \.isHittable, toEqual: true, timeout: 5))
        XCTAssertEqual(field.value as? String, "@sakura ありがとう 🎉")
        capture(app, name: "スタンプを挿入した入力欄")
        app.buttons["send-message"].tap()
        XCTAssertTrue(app.staticTexts["@sakura ありがとう 🎉"].waitForExistence(timeout: 5))
    }

    @MainActor
    func testComposerAdditionsStayAlignedWithMultilineDraft() {
        let app = launchWorkspace()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let field = composer(app)
        field.tap()
        field.typeText("1行目\n2行目\n3行目")
        let additions = app.buttons["composer-additions"]
        let send = app.buttons["send-message"]
        XCTAssertEqual(additions.frame.midY, send.frame.midY, accuracy: 1)
        capture(app, name: "複数行を入力中の追加ボタン")
        app.buttons["dismiss-composer-keyboard"].tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForNonExistence(timeout: 5))
        XCTAssertEqual(additions.frame.midY, send.frame.midY, accuracy: 1)
        capture(app, name: "キーボードを閉じた複数行の追加ボタン")
        additions.tap()
        XCTAssertTrue(app.buttons["composer-attach-file"].waitForExistence(timeout: 5))
    }

    @MainActor
    func testWelcomeStaysPortraitWhenDeviceRotates() throws {
        try XCTSkipUnless(UIDevice.current.userInterfaceIdiom == .phone, "縦向き固定はiPhoneのみ")
        defer { XCUIDevice.shared.orientation = .portrait }
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting", "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"]
        app.launch()
        assertStaysPortraitWhenDeviceRotates(app)
        let login = app.buttons["login"]
        XCTAssertTrue(login.waitForExistence(timeout: 5))
        capture(app, name: "端末を横にしても縦表示を保つログイン画面")
        if !login.isHittable { app.swipeUp() }
        XCTAssertTrue(login.isHittable)
        XCTAssertGreaterThanOrEqual(login.frame.minY, 0)
        XCTAssertLessThanOrEqual(login.frame.maxY, app.frame.height)
    }

    @MainActor
    func testConversationStaysPortraitWhenDeviceRotates() throws {
        try XCTSkipUnless(UIDevice.current.userInterfaceIdiom == .phone, "縦向き固定はiPhoneのみ")
        defer { XCUIDevice.shared.orientation = .portrait }
        let app = launchWorkspace(dark: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let field = composer(app)
        field.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        let draft = "横向きの下書き\n2行目\n3行目\n4行目\n5行目\n6行目"
        // 複数行入力への切り替え完了を待ち、改行の入力順を保つ。
        for (index, line) in draft.split(separator: "\n").enumerated() {
            if index > 0 { field.typeText("\n") }
            field.typeText(String(line))
        }
        XCTAssertEqual(field.value as? String, draft)
        assertStaysPortraitWhenDeviceRotates(app)
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        let list = app.scrollViews["message-list"]
        let send = app.buttons["send-message"]
        let additions = app.buttons["composer-additions"]
        let sendFrame = send.frame
        let additionsFrame = additions.frame
        for frame in [sendFrame, additionsFrame] {
            XCTAssertTrue(app.frame.contains(frame))
            XCTAssertLessThanOrEqual(frame.maxY, app.keyboards.firstMatch.frame.minY)
        }
        XCTAssertGreaterThanOrEqual(list.frame.height, 44)
        assertLatestAboveComposer(app.staticTexts["SwiftUI で、いつもの会話がもっと身近に。"], in: app)
        capture(app, name: "端末を横にしても縦表示を保つ入力中の会話")
        // 実機ではisHittableがfalseでも操作できるため、実際のメニュー表示と送信結果で確認する。
        additions.tap()
        let attachmentAction = app.buttons["composer-attach-file"]
        XCTAssertTrue(attachmentAction.waitForExistence(timeout: 5))
        app.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.2)).tap()
        XCTAssertTrue(attachmentAction.waitForNonExistence(timeout: 5))
        send.tap()
        XCTAssertTrue(list.staticTexts[draft].waitForExistence(timeout: 5))
        XCTAssertFalse(send.isEnabled)
    }

    @MainActor
    private func waitForConversationOrientation(_ app: XCUIApplication) {
        if UIDevice.current.userInterfaceIdiom == .pad { rotateToLandscape(app) }
        else { waitForPortrait(app) }
    }

    @MainActor
    func testIPadWelcomeSupportsRotation() throws {
        try XCTSkipUnless(UIDevice.current.userInterfaceIdiom == .pad, "iPadで回転を確認する")
        defer { XCUIDevice.shared.orientation = .portrait }
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting", "--uitesting-orientation"]
        app.launch()
        let login = app.buttons["login"]
        XCTAssertTrue(login.waitForExistence(timeout: 5))
        rotateToLandscape(app)
        XCTAssertTrue(login.isHittable)
        XCTAssertTrue(app.frame.contains(login.frame))
        capture(app, name: "iPadの横向きログイン画面")
        XCUIDevice.shared.orientation = .portraitUpsideDown
        XCTAssertTrue(app.otherElements["interface-orientation"].wait(for: \.label, toEqual: "portraitUpsideDown", timeout: 5))
        XCTAssertTrue(login.isHittable)
        capture(app, name: "iPadの上下逆向きログイン画面")
    }

    @MainActor
    func testIPadConversationPreservesDraftAcrossRotationAndTabs() throws {
        try XCTSkipUnless(UIDevice.current.userInterfaceIdiom == .pad, "iPadで入力と回転を確認する")
        defer { XCUIDevice.shared.orientation = .portrait }
        let app = launchWorkspace()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let field = composer(app)
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        field.tap()
        field.typeText("iPadの下書き\n回転後も入力を続けます")
        rotateToLandscape(app)
        XCTAssertTrue((field.value as? String)?.contains("iPadの下書き") == true)
        let send = app.buttons["send-message"]
        XCTAssertTrue(send.isHittable)
        XCTAssertTrue(app.frame.contains(send.frame))
        XCTAssertGreaterThan(app.scrollViews["message-list"].frame.height, 44)
        capture(app, name: "iPadの横向き入力画面")
        selectTab("DM", in: app, dismissingKeyboard: true)
        XCTAssertTrue(app.buttons["new-dm"].waitForExistence(timeout: 5))
        selectTab("チャンネル", in: app)
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        XCTAssertTrue((field.value as? String)?.contains("iPadの下書き") == true)
        XCUIDevice.shared.orientation = .portrait
        waitForPortrait(app)
        send.tap()
        XCTAssertTrue(app.staticTexts["iPadの下書き\n回転後も入力を続けます"].waitForExistence(timeout: 5))
        XCTAssertFalse(send.isEnabled)
        capture(app, name: "iPadの縦向き送信後")
    }

    @MainActor
    private func rotateToLandscape(_ app: XCUIApplication) {
        XCUIDevice.shared.orientation = .landscapeLeft
        let rotated = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            app.frame.width > app.frame.height
        }, object: app)
        XCTAssertEqual(XCTWaiter.wait(for: [rotated], timeout: 5), .completed)
    }

    @MainActor
    private func waitForPortrait(_ app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
        let portrait = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            app.frame.height > app.frame.width
        }, object: app)
        XCTAssertEqual(XCTWaiter.wait(for: [portrait], timeout: 5), .completed, file: file, line: line)
    }

    @MainActor
    private func assertStaysPortraitWhenDeviceRotates(_ app: XCUIApplication, file: StaticString = #filePath, line: UInt = #line) {
        waitForPortrait(app, file: file, line: line)
        XCUIDevice.shared.orientation = .landscapeLeft
        let rotated = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            app.frame.width > app.frame.height
        }, object: app)
        rotated.isInverted = true
        XCTAssertEqual(XCTWaiter.wait(for: [rotated], timeout: 3), .completed, file: file, line: line)
    }

    @MainActor
    func testLeftSwipeReopensPreviousChannelWithDraft() throws {
        try XCTSkipUnless(UIDevice.current.userInterfaceIdiom == .phone, "1列表示の一覧でスワイプを確認する")
        let app = launchWorkspace()
        let general = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        let section = app.buttons["channel-section-list"]
        section.tap()
        XCTAssertTrue(general.waitForNonExistence(timeout: 5))
        section.tap()
        XCTAssertTrue(general.waitForExistence(timeout: 5))
        general.tap()
        let field = composer(app)
        field.tap()
        field.typeText("一覧から戻って続ける下書き")
        app.buttons["dismiss-composer-keyboard"].tap()
        app.navigationBars.buttons["BackButton"].tap()
        XCTAssertTrue(general.waitForExistence(timeout: 5))
        general.swipeRight()
        XCTAssertTrue(general.isHittable)
        XCTAssertFalse(field.exists)
        general.swipeUp()
        XCTAssertFalse(field.exists)
        app.collectionViews.firstMatch.swipeLeft()
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        XCTAssertEqual(field.value as? String, "一覧から戻って続ける下書き")
        XCTAssertTrue(app.buttons["channel-details"].isHittable)
    }

    @MainActor
    func testWelcomeHasAccessibleLogin() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting"]
        app.launch()
        XCTAssertTrue(app.buttons["login"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["login"].isEnabled)
        let viewport = app.scrollViews["login-screen"]
        let heading = app.staticTexts["ようこそ Zoo へ"]
        let login = app.buttons["login"]
        XCTAssertEqual(heading.frame.midX, viewport.frame.midX, accuracy: 2)
        XCTAssertEqual(login.frame.midX, viewport.frame.midX, accuracy: 2)
        XCTAssertGreaterThan(heading.frame.minY, viewport.frame.minY + viewport.frame.height / 4)
        XCTAssertLessThan(login.frame.maxY, viewport.frame.maxY - viewport.frame.height / 4)
        capture(app, name: "中央に配置したログイン画面")
    }

    @MainActor
    func testDirectoryLoadingIsCentered() throws {
        try XCTSkipUnless(UIDevice.current.userInterfaceIdiom == .phone, "タブとナビゲーションの間の中央配置を確認する")
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-directory-loading"]
        app.launch()
        let indicator = app.descendants(matching: .any)["directory-loading"].firstMatch
        XCTAssertTrue(indicator.waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["参加中のチャンネルはありません"].exists)
        let top = app.navigationBars.firstMatch.frame.maxY
        let bottom = app.tabBars.firstMatch.frame.minY
        XCTAssertEqual(indicator.frame.midX, app.frame.midX, accuracy: 2)
        XCTAssertEqual(indicator.frame.midY, (top + bottom) / 2, accuracy: 2)
        capture(app, name: "中央に配置した一覧の読み込み表示")
    }

    @MainActor
    func testDirectoryWaitsForPinsAndAllowsUseWhenSyncIsSlow() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-pins-loading", "--uitesting-chat-details"]
        app.launch()
        XCTAssertTrue(app.descendants(matching: .any)["directory-loading"].firstMatch.waitForExistence(timeout: 5))
        XCTAssertFalse(app.buttons["channel-section-list"].exists)
        XCTAssertFalse(app.buttons["channel-section-pinned"].exists)
        XCTAssertFalse(app.staticTexts["参加中のチャンネルはありません"].exists)
        XCTAssertTrue(app.buttons["channel-section-list"].waitForExistence(timeout: 15))
        XCTAssertTrue(app.staticTexts["ピン留めの取得に時間がかかっています。"].exists)
        XCTAssertFalse(app.descendants(matching: .any)["directory-loading"].firstMatch.exists)
        app.descendants(matching: .any)["channel-11111111-1111-4111-8111-111111111111"].firstMatch.tap()
        XCTAssertTrue(app.navigationBars["一般"].waitForExistence(timeout: 5))
    }

    @MainActor
    func testCreatedChannelAppearsAfterInitialReadStateFailure() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-directory-read-error"]
        app.launch()
        XCTAssertTrue(app.staticTexts["操作に失敗しました（503）。参加状態を取得できませんでした。"].waitForExistence(timeout: 5))
        app.buttons["new-conversation"].tap()
        app.buttons["チャンネルを作成"].tap()
        let field = app.textFields["channel-name"]
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        field.tap()
        field.typeText("取得失敗後の会話")
        app.buttons["create-channel"].tap()
        XCTAssertTrue(app.navigationBars["取得失敗後の会話"].waitForExistence(timeout: 5))
        app.navigationBars.buttons.firstMatch.tap()
        let row = app.buttons.containing(.staticText, identifier: "取得失敗後の会話").firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["参加中のチャンネルはありません"].exists)
        row.tap()
        XCTAssertTrue(app.navigationBars["取得失敗後の会話"].waitForExistence(timeout: 5))
    }

    @MainActor
    func testDirectoryWaitsForCachedMemberships() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-directory-cache-loading"]
        app.launch()
        XCTAssertTrue(app.descendants(matching: .any)["directory-loading"].firstMatch.waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["参加中のチャンネルはありません"].exists)
        selectTab("DM", in: app)
        XCTAssertFalse(app.staticTexts["DMはまだありません"].exists)
        XCTAssertTrue(app.descendants(matching: .any)["directory-loading"].firstMatch.waitForExistence(timeout: 5))
    }

    @MainActor
    func testDirectoryFailureDoesNotShowEmptyState() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-directory-error"]
        app.launch()
        XCTAssertTrue(app.staticTexts["操作に失敗しました（503）。チャンネルを取得できませんでした。"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["参加中のチャンネルはありません"].exists)
        XCTAssertFalse(app.descendants(matching: .any)["directory-loading"].firstMatch.exists)
        selectTab("DM", in: app)
        XCTAssertFalse(app.staticTexts["DMはまだありません"].exists)
    }

    @MainActor
    func testDirectoryRetryShowsLoading() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-directory-retry"]
        app.launch()
        let error = app.staticTexts["操作に失敗しました（503）。チャンネルを取得できませんでした。"]
        XCTAssertTrue(error.waitForExistence(timeout: 5))
        XCTAssertTrue(app.descendants(matching: .any)["directory-loading"].firstMatch.waitForExistence(timeout: 35))
        XCTAssertFalse(error.exists)
        XCTAssertFalse(app.staticTexts["参加中のチャンネルはありません"].exists)
    }

    @MainActor
    private func launchWorkspace(failingSend: Bool = false, liveMessage: Bool = false, longHistory: Bool = false,
                                 threadDirectory: Bool = false, chatDetails: Bool = false, video: Bool = false, readRetry: Bool = false, dark: Bool = false, pinFailure: Bool = false, call: Bool = false, attachments: Bool = false,
                                 pdf: Bool = false, pdfFailure: Bool = false, sensitivePDF: Bool = false, callInviteFailure: Bool = false,
                                 tallImage: Bool = false, wrappedReactions: Bool = false, retainedMessages: Bool = false,
                                 variedHistory: Bool = false, longThread: Bool = false, largeText: Bool = false,
                                 shortThread: Bool = false, readSaveRetry: Bool = false, activeCalls: Bool = false, pagedHistory: Bool = false) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace"]
        if failingSend { app.launchArguments.append("--uitesting-send-failure") }
        if liveMessage { app.launchArguments.append("--uitesting-live-message") }
        if longHistory { app.launchArguments.append("--uitesting-long-history") }
        if pagedHistory { app.launchArguments.append("--uitesting-paged-history") }
        if dark { app.launchArguments += ["-zoo.appearance", "dark"] }
        if video { app.launchArguments.append("--uitesting-video") }
        if tallImage { app.launchArguments.append("--uitesting-tall-image") }
        if wrappedReactions { app.launchArguments.append("--uitesting-wrapped-reactions") }
        if retainedMessages { app.launchArguments.append("--uitesting-retained-messages") }
        if variedHistory { app.launchArguments.append("--uitesting-varied-history") }
        if readRetry { app.launchArguments.append("--uitesting-read-retry") }
        if readSaveRetry { app.launchArguments.append("--uitesting-read-save-retry") }
        if pinFailure { app.launchArguments.append("--uitesting-pin-failure") }
        if call { app.launchArguments.append("--uitesting-call") }
        if activeCalls { app.launchArguments.append("--uitesting-active-calls") }
        if callInviteFailure { app.launchArguments.append("--uitesting-call-invite-failure") }
        if attachments { app.launchArguments.append("--uitesting-attachments") }
        if pdf { app.launchArguments.append("--uitesting-pdf") }
        if pdfFailure { app.launchArguments.append("--uitesting-pdf-failure") }
        if sensitivePDF { app.launchArguments.append("--uitesting-sensitive-pdf") }
        if chatDetails { app.launchArguments.append("--uitesting-chat-details") }
        if threadDirectory { app.launchArguments.append("--uitesting-thread-directory") }
        if longThread { app.launchArguments.append("--uitesting-long-thread") }
        if shortThread { app.launchArguments.append("--uitesting-short-thread") }
        if largeText { app.launchArguments += ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"] }
        app.launch()
        XCTAssertTrue(app.buttons["channel-11111111-1111-4111-8111-111111111111"].waitForExistence(timeout: 10))
        return app
    }

    @MainActor
    private func composer(_ app: XCUIApplication, reply: Bool = false) -> XCUIElement {
        let id = reply ? "reply-input" : "message-input"
        // iOSでは入力状態によってTextFieldとTextViewが切り替わるため、識別子で追跡する。
        return app.descendants(matching: .any).matching(NSPredicate(
            format: "identifier == %@ AND (elementType == %d OR elementType == %d)",
            id, XCUIElement.ElementType.textField.rawValue, XCUIElement.ElementType.textView.rawValue
        )).firstMatch
    }

    @MainActor
    private func capture(_ app: XCUIApplication, name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
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
        let tab = tabButton(title, in: app)
        XCTAssertTrue(tab.wait(for: \.isHittable, toEqual: true, timeout: 5))
        tab.tap()
        XCTAssertTrue(tab.wait(for: \.isSelected, toEqual: true, timeout: 5))
    }

    @MainActor
    private func tabButton(_ title: String, in app: XCUIApplication) -> XCUIElement {
        // iPadの上部タブはTabBarとして公開されないため、ボタンから取得する。
        if UIDevice.current.userInterfaceIdiom == .pad {
            return app.buttons.matching(NSPredicate(format: "label == %@ AND identifier != 'BackButton'", title)).firstMatch
        }
        return app.tabBars.buttons[title]
    }

    @MainActor
    func testChannelRemainsResponsiveWithEncryptionController() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-e2ee"]
        app.launch()
        let channel = app.buttons["channel-11111111-1111-4111-8111-111111111111"]
        XCTAssertTrue(channel.waitForExistence(timeout: 10))
        channel.tap()
        let details = app.buttons["channel-details"]
        XCTAssertTrue(details.waitForExistence(timeout: 5))
        details.tap()
        XCTAssertTrue(app.navigationBars["チャンネル詳細"].waitForExistence(timeout: 5))
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
        XCTAssertTrue(["", "#一般 へのメッセージ"].contains(field.value as? String ?? ""))
        XCTAssertFalse(app.buttons["send-message"].isEnabled)
        capture(app, name: "ネイティブチャットと日本語入力")
        openActions(for: app.staticTexts["こんにちは！ Zoo の iOS 版へようこそ。"], in: app)
        app.buttons["message-action-reply"].tap()
        let threadBar = app.navigationBars["スレッド"]
        XCTAssertTrue(threadBar.waitForExistence(timeout: 5))
        XCTAssertFalse(threadBar.buttons["閉じる"].exists)
        XCTAssertTrue(app.staticTexts["thread-empty-state"].exists)
        XCTAssertEqual(app.staticTexts["thread-reply-count"].label, "0 件の返信")
        app.typeText("スレッドへの返信テスト")
        app.buttons["send-reply"].tap()
        XCTAssertTrue(app.staticTexts["スレッドへの返信テスト"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["別の端末からの返信"].waitForExistence(timeout: 10))
        capture(app, name: "スレッドへの返信")
        threadBar.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(app.navigationBars["一般"].waitForExistence(timeout: 5))
        let replies = app.buttons["thread-replies-22222222-2222-4222-8222-222222222222"]
        XCTAssertTrue(replies.waitForExistence(timeout: 5))
        XCTAssertEqual(replies.label, "2 件の返信")
        XCTAssertTrue((replies.value as? String)?.contains("あなた") == true)
        XCTAssertTrue((replies.value as? String)?.contains("さくら") == true)
        XCTAssertTrue((replies.value as? String)?.contains("最終返信") == true)
        XCTAssertTrue(app.staticTexts["別の端末からの新着"].waitForExistence(timeout: 10))
        XCTAssertFalse(app.navigationBars["スレッド"].exists)
        replies.tap()
        XCTAssertTrue(threadBar.waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["スレッドへの返信テスト"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["別の端末からの返信"].exists)
        XCTAssertTrue(app.keyboards.firstMatch.waitForNonExistence(timeout: 5))
        XCTAssertEqual(app.staticTexts["thread-reply-count"].label, "2 件の返信")
        XCTAssertFalse(app.staticTexts["thread-empty-state"].exists)
        XCTAssertFalse(replies.exists)
        capture(app, name: "返信件数から開いたスレッド")
        threadBar.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(replies.waitForExistence(timeout: 5))
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
        app.buttons["search-reaction"].tap()
        let search = app.textFields["reaction-search-input"]
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
        app.buttons["search-reaction"].tap()
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
        let editor = app.descendants(matching: .any).matching(identifier: "edit-message-input").firstMatch
        XCTAssertTrue(editor.waitForExistence(timeout: 5))
        XCTAssertEqual(editor.value as? String, original)
        app.buttons["save-message-edit"].tap()
        XCTAssertTrue(link.waitForExistence(timeout: 5))
        link.tap()
        XCTAssertTrue(app.wait(for: .runningBackground, timeout: 5))
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
    func testNearbyOffscreenAttachmentKeepsItsLoadedState() {
        let app = launchWorkspace(longHistory: true, retainedMessages: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let reveal = app.buttons["センシティブな添付を表示"]
        XCTAssertTrue(reveal.waitForExistence(timeout: 10))
        reveal.tap()
        let preview = app.buttons["PDFを全画面で表示：確認資料.pdf"]
        XCTAssertTrue(preview.waitForExistence(timeout: 10))
        let list = app.scrollViews["message-list"]
        let latest = app.staticTexts["履歴 60"]
        XCTAssertTrue(latest.isHittable)
        // 端末の高さにかかわらず、末尾だけを画面外へ送り、近隣行はキャッシュ範囲に残す。
        let distance = list.frame.maxY + 20 - latest.frame.minY
        let start = list.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: 0.2))
        start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: distance)),
                    withVelocity: .slow, thenHoldForDuration: 0.3)
        XCTAssertFalse(preview.isHittable)
        XCTAssertFalse(latest.isHittable)
        XCTAssertTrue(app.staticTexts["履歴 57"].isHittable)
        let jump = app.buttons["scroll-to-latest"]
        XCTAssertTrue(jump.waitForExistence(timeout: 5))
        jump.tap()
        XCTAssertTrue(preview.wait(for: \.isHittable, toEqual: true, timeout: 5))
        XCTAssertFalse(reveal.exists)
        XCTAssertFalse(app.staticTexts["保持した添付が再読み込みされました。"].exists)
        capture(app, name: "画面外から戻っても読み込み済みの添付を保持")
    }

    @MainActor
    func testJumpToLatestInVariedHistoryKeepsBottomPosition() {
        let app = launchWorkspace(longHistory: true, chatDetails: true, variedHistory: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let latest = app.staticTexts["履歴 60"]
        XCTAssertTrue(latest.waitForExistence(timeout: 10))
        assertLatestAboveComposer(latest, in: app)
        let list = app.scrollViews["message-list"]
        let jump = app.buttons["scroll-to-latest"]
        for _ in 0..<2 {
            for _ in 0..<4 { list.swipeDown(velocity: .fast) }
            XCTAssertFalse(latest.isHittable)
            XCTAssertTrue(jump.waitForExistence(timeout: 5))
            jump.tap()
            XCTAssertTrue(jump.waitForNonExistence(timeout: 5))
            XCTAssertTrue(latest.isHittable)
            let latestY = latest.frame.minY
            assertLatestAboveComposer(latest, in: app)
            let previous = list.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "履歴 59\n")).firstMatch
            XCTAssertTrue(previous.isHittable)
            XCTAssertLessThanOrEqual(previous.frame.maxY, latest.frame.minY, "隣り合う投稿の本文を重ねない")
            capture(app, name: "長文の履歴から最新の投稿へ移動")
            let start = list.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: 0.8))
            start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: -280)),
                        withVelocity: .slow, thenHoldForDuration: 0.3)
            XCTAssertEqual(latest.frame.minY, latestY, accuracy: 2)
        }
    }

    @MainActor
    func testBottomOverscrollKeepsLatestPosition() {
        let app = launchWorkspace(longHistory: true)
        assertBottomOverscrollKeepsPosition(app, latestText: "履歴 60")
    }

    @MainActor
    func testVariedHistoryBottomOverscrollKeepsLatestPosition() {
        let app = launchWorkspace(longHistory: true, variedHistory: true)
        assertBottomOverscrollKeepsPosition(app, latestText: "履歴 60")
    }

    @MainActor
    func testShortConversationBottomOverscrollKeepsPosition() {
        let app = launchWorkspace()
        assertBottomOverscrollKeepsPosition(app, latestText: "SwiftUI で、いつもの会話がもっと身近に。")
    }

    @MainActor
    func testShortConversationKeepsLatestAboveResizingComposer() {
        let app = launchWorkspace()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let latest = app.staticTexts["SwiftUI で、いつもの会話がもっと身近に。"]
        XCTAssertTrue(latest.waitForExistence(timeout: 10))
        assertLatestAboveComposer(latest, in: app)
        capture(app, name: "短い会話も最新投稿を入力欄の直上に配置")
        let field = composer(app)
        field.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        capture(app, name: "キーボードを開いた直後の短い会話")
        assertLatestAboveComposer(latest, in: app)
        field.typeText("一行目\n二行目\n三行目")
        assertLatestAboveComposer(latest, in: app)
        capture(app, name: "入力欄が広がっても最新投稿との間隔を維持")
        app.buttons["dismiss-composer-keyboard"].tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForNonExistence(timeout: 5))
        assertLatestAboveComposer(latest, in: app)
    }

    @MainActor
    func testUnreadLongConversationInitiallyShowsLatestAboveComposer() {
        let app = launchWorkspace(longHistory: true, chatDetails: true, variedHistory: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let latest = app.staticTexts["履歴 60"]
        XCTAssertTrue(latest.waitForExistence(timeout: 10))
        assertLatestAboveComposer(latest, in: app)
        XCTAssertFalse(app.buttons["scroll-to-latest"].exists)
        capture(app, name: "未読がある長い会話も最新投稿から表示")
        app.navigationBars.buttons.firstMatch.tap()
        app.buttons["未読のみ"].tap()
        XCTAssertTrue(app.staticTexts["未読の会話はありません"].waitForExistence(timeout: 5))
    }

    @MainActor
    private func assertLatestAboveComposer(_ latest: XCUIElement, in app: XCUIApplication,
                                          file: StaticString = #filePath, line: UInt = #line) {
        XCTAssertTrue(latest.isHittable, file: file, line: line)
        let bottomGap = app.otherElements["message-composer"].frame.minY - latest.frame.maxY
        XCTAssertGreaterThanOrEqual(bottomGap, 0, file: file, line: line)
        XCTAssertLessThan(bottomGap, 60, "最新の投稿を入力欄の直上に揃える", file: file, line: line)
    }

    @MainActor
    private func assertBottomOverscrollKeepsPosition(_ app: XCUIApplication, latestText: String) {
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let latest = app.staticTexts[latestText]
        XCTAssertTrue(latest.waitForExistence(timeout: 10))
        assertLatestAboveComposer(latest, in: app)
        let originalY = latest.frame.minY
        let list = app.scrollViews["message-list"]
        for _ in 0..<3 {
            let start = list.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: 0.8))
            start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: -280)),
                        withVelocity: .slow, thenHoldForDuration: 0.3)
            XCTAssertTrue(latest.isHittable)
            XCTAssertEqual(latest.frame.minY, originalY, accuracy: 2)
            XCTAssertFalse(app.buttons["scroll-to-latest"].exists)
        }
        capture(app, name: "最下部での連続スクロール後も最新位置を維持")
    }

    @MainActor
    func testScrollingLoadsOlderMessagesWithoutButton() {
        let app = launchWorkspace(longHistory: true, pagedHistory: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        assertScrollLoadsHistory(app, listID: "message-list", prefix: "履歴 ")
    }

    @MainActor
    func testScrollingLoadsOlderThreadRepliesWithoutButton() {
        let app = launchWorkspace(longThread: true, pagedHistory: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let link = app.buttons["thread-replies-22222222-2222-4222-8222-222222222222"]
        XCTAssertTrue(link.waitForExistence(timeout: 5))
        XCTAssertEqual(link.label, "60 件の返信")
        link.tap()
        assertScrollLoadsHistory(app, listID: "thread-message-list", prefix: "スレッドの返信 ")
    }

    @MainActor
    private func assertScrollLoadsHistory(_ app: XCUIApplication, listID: String, prefix: String) {
        let list = app.scrollViews[listID]
        let latest = list.staticTexts[prefix + "60"]
        XCTAssertTrue(latest.wait(for: \.isHittable, toEqual: true, timeout: 10))
        XCTAssertFalse(app.buttons["load-older-messages"].exists)
        XCTAssertFalse(list.staticTexts[prefix + "1"].exists)
        let loading = app.descendants(matching: .any)["loading-older-messages"].firstMatch
        XCTAssertFalse(loading.exists)
        for _ in 0..<18 {
            if loading.exists { break }
            list.swipeDown(velocity: .slow)
        }
        XCTAssertTrue(loading.waitForExistence(timeout: 2))
        guard let candidate = list.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", prefix))
            .allElementsBoundByIndex.first(where: { $0.isHittable && $0.frame.minY > list.frame.minY + 20 }) else {
            XCTFail("追加取得前に表示中のメッセージを取得できません")
            return
        }
        // 履歴の追加で配列の添字が変わるため、同じ投稿の識別子を基準に検証する。
        let anchor = list.staticTexts[candidate.identifier]
        let originalY = anchor.frame.minY
        let resume = app.buttons["preview-release-history"]
        XCTAssertTrue(resume.wait(for: \.isEnabled, toEqual: true, timeout: 5))
        resume.tap()
        XCTAssertTrue(loading.waitForNonExistence(timeout: 8))
        XCTAssertTrue(anchor.isHittable)
        XCTAssertEqual(anchor.frame.minY, originalY, accuracy: 4, "追加取得でも読んでいた位置を維持する")
        for _ in 0..<8 where !list.staticTexts[prefix + "1"].isHittable { list.swipeDown(velocity: .slow) }
        XCTAssertTrue(list.staticTexts[prefix + "1"].isHittable)
        XCTAssertFalse(loading.exists)
        capture(app, name: "スクロールによる過去のメッセージ取得")
    }

    @MainActor
    func testReturningFromThreadPreservesHistoryPosition() {
        let app = launchWorkspace(longHistory: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let list = app.scrollViews["message-list"]
        XCTAssertTrue(app.staticTexts["履歴 60"].waitForExistence(timeout: 10))
        list.swipeDown(velocity: .slow)
        list.swipeDown(velocity: .slow)
        let rows = list.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "履歴 "))
        guard let candidate = rows.allElementsBoundByIndex.first(where: {
            $0.isHittable && $0.frame.minY > list.frame.minY + 40 && $0.frame.maxY < list.frame.maxY - 40
        }) else { XCTFail("読み返す投稿が表示されていません。"); return }
        let anchor = app.staticTexts[candidate.identifier]
        let originalY = anchor.frame.minY
        openActions(for: anchor, in: app)
        app.buttons["message-action-reply"].tap()
        let thread = app.navigationBars["スレッド"]
        XCTAssertTrue(thread.waitForExistence(timeout: 5))
        if app.keyboards.firstMatch.waitForExistence(timeout: 2) { app.buttons["dismiss-composer-keyboard"].tap() }
        thread.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(app.navigationBars["一般"].waitForExistence(timeout: 5))
        XCTAssertTrue(anchor.wait(for: \.isHittable, toEqual: true, timeout: 5))
        XCTAssertEqual(anchor.frame.minY, originalY, accuracy: 4)
        capture(app, name: "スレッドから戻っても読み返し位置を維持")
    }

    @MainActor
    func testHistoryRemainsScrollableAfterKeyboardDismissal() {
        let app = launchWorkspace(longHistory: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let latest = app.staticTexts["履歴 60"]
        XCTAssertTrue(latest.waitForExistence(timeout: 10))
        composer(app).tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        assertLatestAboveComposer(latest, in: app)
        let list = app.scrollViews["message-list"]
        let start = list.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: 0)).withOffset(CGVector(dx: 0, dy: 20))
        // 端末サイズによらずキーボードを閉じられるよう、十分な移動量を確保する。
        let distance = min(app.keyboards.firstMatch.frame.height + 300, app.frame.maxY - start.screenPoint.y - 20)
        start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: distance)),
                    withVelocity: .slow, thenHoldForDuration: 0.3)
        XCTAssertTrue(app.keyboards.firstMatch.waitForNonExistence(timeout: 5))
        list.swipeDown(velocity: .slow)
        XCTAssertFalse(latest.isHittable)
        XCTAssertTrue(list.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "履歴 "))
            .allElementsBoundByIndex.contains(where: { $0.isHittable }))
        capture(app, name: "キーボードを閉じた後に過去の会話を読み返す")
    }

    @MainActor
    func testIncomingMessagePreservesPositionWhileReadingHistory() {
        let app = launchWorkspace(liveMessage: true, longHistory: true, shortThread: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        openActions(for: app.staticTexts["履歴 60"], in: app)
        app.buttons["message-action-reply"].tap()
        let thread = app.navigationBars["スレッド"]
        XCTAssertTrue(thread.waitForExistence(timeout: 5))
        composer(app, reply: true).typeText("新着受信前の返信")
        app.buttons["send-reply"].tap()
        thread.buttons.firstMatch.tap()
        let list = app.scrollViews["message-list"]
        for _ in 0..<2 { list.swipeDown(velocity: .fast) }
        let jump = app.buttons["scroll-to-latest"]
        XCTAssertTrue(jump.waitForExistence(timeout: 5))
        guard let candidate = list.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "履歴 "))
            .allElementsBoundByIndex.first(where: { $0.isHittable }) else {
            return XCTFail("読み返す履歴が表示されていません")
        }
        let anchor = list.staticTexts[candidate.identifier]
        let originalY = anchor.frame.minY
        let previousCount = list.value as? String
        let release = app.buttons["preview-release-message"]
        release.tap()
        XCTAssertTrue(release.wait(for: \.label, toEqual: "配信済み", timeout: 5))
        let updated = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in
            guard let count = list.value as? String else { return false }
            return count != previousCount
        }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [updated], timeout: 5), .completed)
        XCTAssertTrue(anchor.exists && anchor.isHittable)
        XCTAssertEqual(anchor.frame.minY, originalY, accuracy: 2)
        capture(app, name: "新着を受信しても読み返し中の位置を維持")
        jump.tap()
        XCTAssertTrue(app.staticTexts["別の端末からの新着"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["別の端末からの新着"].isHittable)
    }

    @MainActor
    func testLongHistoryInitiallyShowsLatestMessage() {
        let app = launchWorkspace(longHistory: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let latest = app.staticTexts["履歴 60"]
        XCTAssertTrue(latest.waitForExistence(timeout: 10))
        XCTAssertTrue(latest.isHittable)
        let jump = app.buttons["scroll-to-latest"]
        XCTAssertFalse(jump.exists)
        let latestY = latest.frame.minY
        let list = app.scrollViews["message-list"]
        let shortStart = list.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: 0.4))
        shortStart.press(forDuration: 0.05, thenDragTo: shortStart.withOffset(CGVector(dx: 0, dy: 120)),
                         withVelocity: .slow, thenHoldForDuration: 0.3)
        XCTAssertGreaterThan(latest.frame.minY - latestY, 80)
        XCTAssertLessThan(latest.frame.minY - latestY, 240)
        XCTAssertFalse(jump.exists)
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.35))
        let end = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.8))
        // カードの押下表示が出てから動かしても、長押しメニューを開く前ならスクロールできる。
        start.press(forDuration: 0.25, thenDragTo: end)
        XCTAssertFalse(latest.isHittable)
        XCTAssertFalse(app.buttons["message-action-reply"].exists)
        XCTAssertTrue(jump.waitForExistence(timeout: 5))
        capture(app, name: "十分に遡ったときだけ表示する最新メッセージボタン")
        jump.tap()
        XCTAssertTrue(jump.waitForNonExistence(timeout: 5))
        XCTAssertTrue(latest.isHittable)
        XCTAssertEqual(latest.frame.minY, latestY, accuracy: 2)
    }

    @MainActor
    func testEditOwnMessage() {
        let app = launchWorkspace()
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let message = app.staticTexts["message-content-66666666-6666-4666-8666-666666666666"]
        openActions(for: message, in: app)
        XCTAssertTrue(app.buttons["message-action-delete"].exists)
        app.buttons["message-action-edit"].tap()
        let field = app.descendants(matching: .any).matching(identifier: "edit-message-input").firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        field.tap()
        field.typeText("（編集確認）")
        app.buttons["save-message-edit"].tap()
        let edited = NSPredicate(format: "label CONTAINS %@", "編集確認")
        XCTAssertEqual(XCTWaiter.wait(for: [XCTNSPredicateExpectation(predicate: edited, object: message)], timeout: 5), .completed)
        XCTAssertTrue(app.staticTexts["編集済み"].waitForExistence(timeout: 5))
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
        capture(app, name: "プロフィールの外部リンク")
        app.buttons["workspace-settings"].tap()
        let logout = app.buttons["logout"]
        if !logout.isHittable { app.swipeUp() }
        XCTAssertTrue(logout.wait(for: \.isHittable, toEqual: true, timeout: 5))
        capture(app, name: "設定の外部リンク")
        let sourceFrame = logout.frame
        logout.tap()
        let confirmation = app.buttons.matching(NSPredicate(format: "label == 'ログアウト' AND identifier != 'logout'")).firstMatch
        XCTAssertTrue(confirmation.waitForExistence(timeout: 5))
        let title = app.staticTexts["ログアウトしますか？"]
        let dialogFrame = title.frame.union(confirmation.frame)
        XCTAssertTrue(dialogFrame.insetBy(dx: -32, dy: -32).intersects(sourceFrame), "確認表示を押したボタンの位置に表示する")
        capture(app, name: "ログアウトボタンに紐づく確認表示")
        app.navigationBars["設定"].staticTexts["設定"].tap()
        XCTAssertTrue(confirmation.waitForNonExistence(timeout: 5))
        XCTAssertTrue(logout.isEnabled)
        logout.tap()
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
        XCTAssertEqual(app.staticTexts["thread-channel-name"].label, "#一般")
        let parent = app.staticTexts["message-content-22222222-2222-4222-8222-222222222222"]
        let count = app.staticTexts["thread-reply-count"]
        let reply = app.staticTexts["参加中のスレッドです"]
        XCTAssertTrue(parent.isHittable)
        XCTAssertEqual(count.label, "1 件の返信")
        XCTAssertLessThan(parent.frame.maxY, count.frame.minY)
        XCTAssertLessThan(count.frame.maxY, reply.frame.minY)
        XCTAssertLessThan(parent.frame.minY, app.frame.height * 0.25)
        XCTAssertTrue(app.keyboards.firstMatch.waitForNonExistence(timeout: 5))
        capture(app, name: "親投稿と返信を区切ったスレッド")
        thread.buttons.element(boundBy: 0).tap()
        let channel = app.navigationBars["一般"]
        XCTAssertTrue(channel.waitForExistence(timeout: 5))
        channel.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(entry.waitForExistence(timeout: 5))
    }

    @MainActor
    func testThreadKeepsLatestWhenKeyboardAndRepliesReduceSpace() {
        let app = launchWorkspace(liveMessage: true, shortThread: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let parent = app.staticTexts["message-content-22222222-2222-4222-8222-222222222222"]
        openActions(for: parent, in: app)
        app.buttons["message-action-reply"].tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        let latest = app.staticTexts["スレッドの返信 4"]
        XCTAssertTrue(latest.wait(for: \.isHittable, toEqual: true, timeout: 5))
        let input = composer(app, reply: true)
        capture(app, name: "キーボード表示後も最新返信が見えるスレッド")
        XCTAssertLessThanOrEqual(latest.frame.maxY, input.frame.minY)
        let content = "境界の確認\n2行目\n3行目"
        app.typeText(content)
        XCTAssertTrue(latest.wait(for: \.isHittable, toEqual: true, timeout: 5))
        XCTAssertLessThanOrEqual(latest.frame.maxY, input.frame.minY)
        app.buttons["send-reply"].tap()
        XCTAssertTrue(app.staticTexts[content].wait(for: \.isHittable, toEqual: true, timeout: 5))
        app.buttons["dismiss-composer-keyboard"].tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForNonExistence(timeout: 5))
        XCTAssertTrue(parent.wait(for: \.isHittable, toEqual: true, timeout: 5))
        let incoming = app.staticTexts["別の端末からの返信\n追加の確認事項です。\n返信が増えても\n最新の内容を\n入力欄の上に\n表示します。"]
        XCTAssertTrue(incoming.waitForExistence(timeout: 15))
        XCTAssertTrue(incoming.wait(for: \.isHittable, toEqual: true, timeout: 5))
        XCTAssertLessThanOrEqual(incoming.frame.maxY, input.frame.minY)
        capture(app, name: "新着で表示領域を超えても最新返信に追従")
    }

    @MainActor
    func testThreadReplyLinkSupportsAccessibilityTextSize() {
        let app = launchWorkspace(longThread: true, largeText: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let link = app.buttons["thread-replies-22222222-2222-4222-8222-222222222222"]
        let list = app.scrollViews["message-list"]
        let input = composer(app)
        for _ in 0..<6 {
            if link.exists && link.frame.maxY > input.frame.minY {
                list.swipeUp()
            } else if link.exists && link.isHittable {
                break
            } else {
                list.swipeDown()
            }
        }
        XCTAssertTrue(link.isHittable)
        XCTAssertLessThanOrEqual(link.frame.maxX, app.frame.maxX)
        XCTAssertEqual(link.label, "30 件の返信")
        XCTAssertTrue((link.value as? String)?.contains("最終返信") == true)
        capture(app, name: "文字を拡大した返信リンク")
        link.tap()
        XCTAssertTrue(app.navigationBars["スレッド"].waitForExistence(timeout: 5))
        XCTAssertTrue(composer(app, reply: true).exists)
    }

    @MainActor
    func testLongThreadCanScrollBetweenParentAndLatestReply() {
        let app = launchWorkspace(dark: true, longThread: true)
        app.buttons["channel-11111111-1111-4111-8111-111111111111"].tap()
        let link = app.buttons["thread-replies-22222222-2222-4222-8222-222222222222"]
        XCTAssertTrue(link.waitForExistence(timeout: 5))
        XCTAssertEqual(link.label, "30 件の返信")
        capture(app, name: "参加者と最終返信を表示するスレッド入口")
        link.tap()
        let latest = app.staticTexts["スレッドの返信 30"]
        XCTAssertTrue(latest.wait(for: \.isHittable, toEqual: true, timeout: 5))
        let parent = app.staticTexts["message-content-22222222-2222-4222-8222-222222222222"]
        let list = app.scrollViews["thread-message-list"]
        for _ in 0..<12 {
            if parent.isHittable { break }
            list.swipeDown()
        }
        XCTAssertTrue(parent.isHittable)
        XCTAssertEqual(app.staticTexts["thread-reply-count"].label, "30 件の返信")
        capture(app, name: "長いスレッドの親投稿と返信の区切り")
        let latestButton = app.buttons["scroll-to-latest"]
        XCTAssertTrue(latestButton.waitForExistence(timeout: 5))
        latestButton.tap()
        XCTAssertTrue(latest.wait(for: \.isHittable, toEqual: true, timeout: 5))
    }

    @MainActor
    func testInboxReturnsToOriginalPositionAfterRefresh() {
        assertInboxReturnsToOriginalPositionAfterRefresh(long: true)
    }

    @MainActor
    func testShortInboxReturnsToOriginalPositionAfterRefresh() {
        assertInboxReturnsToOriginalPositionAfterRefresh(long: false)
    }

    @MainActor
    private func assertInboxReturnsToOriginalPositionAfterRefresh(long: Bool) {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting-workspace", "--uitesting-inbox-refresh"]
        if long { app.launchArguments.append("--uitesting-long-inbox") }
        app.launch()
        XCTAssertTrue(app.buttons["channel-11111111-1111-4111-8111-111111111111"].waitForExistence(timeout: 10))
        selectTab("通知", in: app)
        let mention = app.buttons["inbox-open-22222222-2222-4222-8222-222222222222"]
        XCTAssertTrue(mention.waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["inbox-read-all"].waitForNonExistence(timeout: 5))
        let countText = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "通知の取得：")).firstMatch
        XCTAssertTrue(countText.waitForExistence(timeout: 5))
        guard let initialCount = Int(countText.label.filter(\.isNumber)) else {
            XCTFail("通知の初期取得回数を読み取れませんでした。")
            return
        }
        let originalY = mention.frame.minY
        let list = app.descendants(matching: .any)["inbox-list"].firstMatch
        XCTAssertTrue(list.exists)
        capture(app, name: "通知一覧の更新前")

        for refresh in 1...3 {
            let start = list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2))
            let end = list.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.8))
            start.press(forDuration: 0.1, thenDragTo: end)
            XCTAssertTrue(app.staticTexts["通知の取得：\(initialCount + refresh)回目"].waitForExistence(timeout: 5))
            let returned = XCTNSPredicateExpectation(
                predicate: NSPredicate { _, _ in abs(mention.frame.minY - originalY) < 2 }, object: nil
            )
            XCTAssertEqual(XCTWaiter.wait(for: [returned], timeout: 5), .completed,
                           "更新後は通知行が元の縦位置に戻る必要があります。更新前：\(originalY)、更新後：\(mention.frame.minY)")
            capture(app, name: "通知一覧の更新後")
        }
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
        capture(app, name: "共通の余白と区切り線に揃えた通知一覧")
        mention.tap()
        XCTAssertTrue(app.navigationBars["一般"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["message-content-22222222-2222-4222-8222-222222222222"].exists)
        XCTAssertTrue(tabButton("チャンネル", in: app).isSelected)
    }

    @MainActor
    func testEditPublicProfile() {
        let app = launchWorkspace()
        selectTab("プロフィール", in: app)
        let edit = app.buttons["profile-edit"]
        XCTAssertTrue(edit.waitForExistence(timeout: 5))
        capture(app, name: "共通の一覧表示に揃えたプロフィール")
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
