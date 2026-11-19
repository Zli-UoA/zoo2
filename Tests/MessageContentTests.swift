import Foundation
import SwiftUI
import Testing
@testable import Zoo

@MainActor @Suite("画面外のメッセージ保持")
struct MessageRenderCacheTests {
    @Test("表示範囲の上下5件を同じ状態のまま保持し、それより遠い行を解放する")
    func fiveNeighbors() {
        let cache = MessageRenderCache()
        let ids = (0..<30).map(String.init)
        let original = Dictionary(uniqueKeysWithValues: ids.map { ($0, cache.state(for: $0)) })
        cache.retain(visibleIDs: ["10", "11", "12"], orderedIDs: ids)
        #expect(Set(cache.states.keys) == Set(ids[5...17]))
        for id in ids[5...17] { #expect(cache.state(for: id) === original[id]) }
        #expect(cache.state(for: "4") !== original["4"])
        cache.retain(visibleIDs: ["11", "12", "13"], orderedIDs: ids)
        #expect(Set(cache.states.keys) == Set(ids[6...18]).subtracting(["18"]))
        #expect(cache.state(for: "12") === original["12"])
    }

    @Test("先頭・末尾で存在しない隣接行を作らない", arguments: [0, 9])
    func ends(_ index: Int) {
        let cache = MessageRenderCache()
        let ids = (0..<10).map(String.init)
        for id in ids { _ = cache.state(for: id) }
        cache.retain(visibleIDs: [ids[index]], orderedIDs: ids)
        let expected = index == 0 ? ids[0...5] : ids[4...9]
        #expect(Set(cache.states.keys) == Set(expected))
    }

    @Test("一時的な空の可視範囲では状態を捨てず、削除されたメッセージは解放する")
    func emptyVisibilityAndDeletion() {
        let cache = MessageRenderCache()
        let original = cache.state(for: "message")
        cache.retain(visibleIDs: [], orderedIDs: ["message"])
        #expect(cache.state(for: "message") === original)
        cache.updateMessages([])
        #expect(cache.states.isEmpty)
    }

    @Test("添付の公開状態と画像を保持し、添付が変更・削除された場合だけ作り直す")
    func attachmentState() {
        let message = MessageRenderState()
        var attachment = MessageAttachment(id: "image", file_name: "画像.png", content_type: "image/png")
        let original = message.attachment(attachment)
        original.revealed = true
        #expect(message.attachment(attachment) === original)
        attachment.is_sensitive = true
        let sensitive = message.attachment(attachment)
        #expect(sensitive !== original)
        #expect(!sensitive.revealed)
        message.retainAttachments([])
        #expect(message.attachment(attachment) !== sensitive)
    }

    @Test("画像とPDFは再表示しても取得・描画し直さず、明示した再試行では更新する", arguments: [false, true])
    func reusesThumbnail(pdf: Bool) async throws {
        let client = PreviewChatClient()
        let workspace = WorkspaceStore(client: client, user: client.currentUser, recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let attachment = MessageAttachment(id: pdf ? "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa03" : "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaa04",
                                           file_name: pdf ? "確認資料.pdf" : "縦長画像.png",
                                           content_type: pdf ? "application/pdf" : "image/png", is_sensitive: true)
        let state = AttachmentRenderState(attachment: attachment)
        await state.load(workspace: workspace)
        if pdf { #expect(state.url == nil) }
        else { #expect(state.url != nil && state.image != nil && !state.revealed) }
        let preloaded = state.image
        state.revealed = true
        await state.load(workspace: workspace)
        let original = try #require(pdf ? state.pdfThumbnail : state.image)
        if !pdf { #expect(original === preloaded) }
        #expect(max(original.size.width, original.size.height) <= 1200)
        state.cancelLoad()
        await state.load(workspace: workspace)
        #expect((pdf ? state.pdfThumbnail : state.image) === original)
        state.retryLoading()
        await state.load(workspace: workspace)
        #expect((pdf ? state.pdfThumbnail : state.image) !== original)
        #expect(state.error == nil)
    }

    @Test("キャンセル済みの行タスクは通信を始めず、保持した本文画像も変更しない")
    func cancelledCaller() async throws {
        let harness = try ChatHarness()
        defer { harness.close() }
        harness.stub { _ in
            Issue.record("キャンセル済みの行から通信が開始されました")
            return (500, [:], Data())
        }
        let workspace = WorkspaceStore(client: harness.client, user: Person(id: "you", user_name: "you", display_name: "あなた"), recentEmojiDefaults: nil)
        defer { workspace.shutdown() }
        let attachment = AttachmentRenderState(attachment: MessageAttachment(id: "attachment", file_name: "資料.txt", content_type: "text/plain"))
        let message = MessageRenderState()
        let url = try #require(URL(string: "https://example.org/stamp.png"))
        let image = UIImage()
        message.inlineImages[url] = image
        await Task { @MainActor in
            withUnsafeCurrentTask { $0?.cancel() }
            await attachment.load(workspace: workspace)
            await message.loadInlineImages([])
        }.value
        #expect(attachment.url == nil && attachment.error == nil)
        #expect(message.inlineImages[url] === image)
    }
}

@Suite("メッセージ本文のリンクとスタンプ")
struct MessageContentTests {
    @Test("日本語・改行・複数のURLを保ち、句読点をリンクに含めない")
    func links() throws {
        let first = try #require(URL(string: "https://example.com/a?q=1&b=2#section"))
        let second = try #require(URL(string: "http://example.org"))
        let content = "確認（\(first.absoluteString)）。\n次は \(second.absoluteString) です"
        let parts = MessageContent.parse(content) { _ in nil }
        #expect(parts == [.text("確認（"), .link(first.absoluteString, first), .text("）。\n次は "),
                          .link(second.absoluteString, second), .text(" です")])
    }

    @Test("URL内の短縮名を変換せず、隣接する日本語名・別名を画像にする")
    func inlineEmojiAndLink() throws {
        let image = try #require(URL(string: "https://example.org/stamp.png"))
        let link = try #require(URL(string: "https://example.org/:smile:/?name=:草:"))
        let parts = MessageContent.parse("前:草::kusa: \(link.absoluteString) 後") {
            [":草:", ":kusa:"].contains($0) ? image : nil
        }
        #expect(parts == [.text("前"), .emoji(":草:", image), .emoji(":kusa:", image), .text(" "),
                          .link(link.absoluteString, link), .text(" 後")])
    }

    @Test("標準短縮名を描画し、同名のカスタム画像を優先する")
    func standardAndCustom() throws {
        #expect(MessageContent.parse(":smile::+1:") { _ in nil } == [.text("😄"), .text("👍")])
        let url = try #require(URL(string: "https://example.org/smile.png"))
        #expect(MessageContent.parse(":smile:") { _ in url } == [.emoji(":smile:", url)])
    }

    @Test("時刻や句読点のコロンが後続スタンプの開始位置を奪わない", arguments: [
        ("時刻12:30、:smile:", "時刻12:30、"), ("注意:（:smile:", "注意:（"),
        (":unknown:smile:", ":unknown")
    ])
    func overlappingUnknownCandidate(_ content: String, _ prefix: String) {
        #expect(MessageContent.parse(content) { _ in nil } == [.text(prefix), .text("😄")])
    }

    @Test("認識済みスタンプの終端コロンを次の開始位置に流用しない")
    func overlappingKnownCandidate() {
        #expect(MessageContent.parse(":smile:+1:") { _ in nil } == [.text("😄"), .text("+1:")])
    }

    @Test("未登録名・空白・コード・許可していないURLは原文のまま残す", arguments: [
        "未登録 :unknown_stamp: :空 白: 12:30", "`https://example.com :smile:`",
        "```\n:smile: https://example.com\n```", "javascript:alert(1)",
        "mailto:person@example.com", "ftp://example.com", "https://user:password@example.com"
    ])
    func literal(_ text: String) {
        #expect(MessageContent.parse(text) { _ in nil } == [.text(text)])
    }

    @Test("空の本文と絵文字を含む長文を処理できる")
    func longContent() {
        #expect(MessageContent.parse("") { _ in nil }.isEmpty)
        let prefix = String(repeating: "😀日本語\n", count: 6_000)
        #expect(MessageContent.parse(prefix + ":smile:") { _ in nil } == [.text(prefix), .text("😄")])
    }
}

@MainActor
@Suite("本文とリアクションの画像解決")
struct MessageEmojiImageTests {
    @Test("上限近い本文でも短縮名や画像の多さで描画が落ちない", arguments: [false, true])
    func denseContentRenders(custom: Bool) throws {
        let url = try #require(URL(string: "https://example.org/stamp.png"))
        let image = UIGraphicsImageRenderer(size: CGSize(width: 32, height: 32)).image { context in
            UIColor.systemPink.setFill()
            context.fill(CGRect(x: 0, y: 0, width: 32, height: 32))
        }
        let parts = MessageContent.parse(String(repeating: ":smile:", count: 5_700)) { _ in custom ? url : nil }
        let content = MessageContentText.render(parts, images: custom ? [url: image] : [:], emojiHeight: 24)
            .font(.body).frame(width: 320, height: 600, alignment: .topLeading).clipped()
        let rendered = try #require(ImageRenderer(content: content).uiImage)
        #expect(rendered.size == CGSize(width: 320, height: 600))
    }

    @Test("本文の別名は添付された正式名の画像を優先し、未登録名も表示する")
    func attachedImages() async throws {
        let client = PreviewChatClient()
        let workspace = WorkspaceStore(client: client, user: client.currentUser, recentEmojiDefaults: nil)
        await workspace.loadEmojis()
        let message = Message(id: "m", user_id: "u", channel_id: "c", content: ":hana: :remote:", created_at: "",
                              display_data: MessageDisplayData(emojis: ["sakura": DisplayEmoji(url: "/stamp.png"),
                                                                        "remote": DisplayEmoji(url: "https://other.invalid/stamp.png")]))
        #expect(workspace.emojiImageURL(for: ":hana:", message: message)?.absoluteString == "https://ui.test.invalid/stamp.png")
        #expect(workspace.emojiImageURL(for: ":remote:", message: message)?.absoluteString == "https://other.invalid/stamp.png")
        #expect(workspace.emojiImageURL(for: ":missing:", message: message) == nil)
    }
}
