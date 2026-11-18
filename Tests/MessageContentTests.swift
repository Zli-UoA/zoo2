import Foundation
import SwiftUI
import Testing
@testable import Zoo

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
