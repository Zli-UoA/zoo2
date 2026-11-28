import Foundation
import Testing
import WebKit
import UIKit
@testable import Zoo

@MainActor @Suite("MFMの描画", .timeLimit(.minutes(1)))
struct MfmTests {
    private func payload(_ content: String) -> MfmPayload {
        MfmPayload(content: content, emojis: [":smile:": "😄"], mentions: ["you": true], channels: [:],
                   restrictedMentions: false, dark: false, reducedMotion: false, active: true, fontSize: 17)
    }

    @Test("同梱した文書で装飾・数式・コード・保護領域を描画する")
    func bundledRenderer() async throws {
        let model = MfmPageModel()
        await model.render(payload("# 見出し\n**太字** <i>斜体</i> ~~取消~~ :smile: @you\n$[ruby 漢字 かんじ] $[spin.speed=2s 回転]\n\\(x^2\\)\n```swift\nlet x = 1\n```\n<plain>**原文** :smile:</plain>"))
        #expect(!model.failed)
        #expect(model.height > 24)
        let result = try await model.page.callJavaScript("return [document.querySelector('h1')?.textContent, document.querySelector('strong')?.textContent, document.querySelector('rt')?.textContent, document.querySelector('.mfm-spin')?.style.animationDuration, !!document.querySelector('.katex'), !!document.querySelector('.hljs-keyword'), document.querySelector('.mention-self')?.textContent, document.body.textContent.includes('**原文** :smile:')];") as? [Any]
        let values = try #require(result)
        #expect(values[0] as? String == "見出し")
        #expect(values[1] as? String == "太字")
        #expect(values[2] as? String == "かんじ")
        #expect(values[3] as? String == "2s")
        #expect(values[4] as? Bool == true)
        #expect(values[5] as? Bool == true)
        #expect(values[6] as? String == "@you")
        #expect(values[7] as? Bool == true)
    }

    @Test("本文を実行せず、外部通信と危険なリンクを禁止する")
    func security() async throws {
        let model = MfmPageModel()
        await model.render(payload("<script>window.injected=true</script> [危険](javascript:alert(1)) <img src=https://evil.invalid/x onerror=alert(1)> \\(\\href{javascript:alert(1)}{危険}\\)"))
        #expect(!model.failed)
        let result = try await model.page.callJavaScript("try { await fetch('https://evil.invalid/'); return false; } catch { return !window.injected && !document.querySelector('#content script, #content img, a[href^=\"javascript:\"]'); }") as? Bool
        #expect(result == true)
    }

    @Test("編集後に短くなった本文の高さとテーマ・動きの設定を更新する")
    func updates() async throws {
        let model = MfmPageModel()
        await model.render(payload(String(repeating: "行\n", count: 30)))
        let original = model.height
        var changed = payload("$[spin 回転] **短文**")
        changed.dark = true
        changed.reducedMotion = true
        changed.active = false
        await model.render(changed)
        #expect(!model.failed)
        #expect(model.height < original)
        let result = try await model.page.callJavaScript("return document.documentElement.classList.contains('dark') && getComputedStyle(document.querySelector('.mfm-spin')).animationName === 'none';") as? Bool
        #expect(result == true)
    }

    @Test("同じ接続先のチャンネル・返信リンクだけをアプリ内の行き先にする")
    func routing() throws {
        let channel = "11111111-1111-4111-8111-111111111111"
        let message = "22222222-2222-4222-8222-222222222222"
        let url = try #require(URL(string: "https://zli.chat/channels/\(channel)?thread=\(message)"))
        let target = try #require(MfmMessageView.channelTarget(url, serverURL: "https://zli.chat:443"))
        #expect(target.channelID == channel && target.messageID == message)
        #expect(MfmMessageView.channelTarget(url, serverURL: "https://other.invalid") == nil)
        #expect(MfmMessageView.channelTarget(url, serverURL: "http://zli.chat") == nil)
    }

    @Test("装飾の内側でもメンションの許可を守り、plainでは画像を取得しない")
    func symbols() {
        #expect(MessageContent.mfmMentions(in: "**@you** $[tada @other]", resolve: { $0 == "you" ? true : nil }) == ["you": true])
        #expect(MessageContent.parse("<plain>:smile: https://example.com</plain>") { _ in URL(string: "https://example.com/stamp.png") } == [.text("<plain>:smile: https://example.com</plain>")])
        #expect(MessageContent.mfmMentions(in: "**@you.**", resolve: { $0 == "you" ? true : nil }) == ["you": true])
    }

    @Test("装飾中の日本語名スタンプを表示し、plainとコードは置換しない")
    func japaneseEmoji() async throws {
        let model = MfmPageModel()
        var value = payload("**:草:** [確認 :草:](https://example.com) <plain>:草:</plain> `:草:`")
        let image = UIGraphicsImageRenderer(size: CGSize(width: 2, height: 2)).image { context in
            UIColor.systemGreen.setFill()
            context.fill(CGRect(x: 0, y: 0, width: 2, height: 2))
        }
        let data = try #require(image.pngData())
        value.emojis[":草:"] = "data:image/png;base64," + data.base64EncodedString()
        await model.render(value)
        #expect(!model.failed)
        let result = try await model.page.callJavaScript("return document.querySelectorAll('img').length === 2 && document.querySelector('strong img')?.alt === ':草:' && document.querySelector('a img')?.alt === ':草:' && document.querySelector('code')?.textContent === ':草:';") as? Bool
        #expect(result == true)
    }

    @Test("MFMで見えるスタンプだけを画像の取得対象にする")
    func emojiDiscovery() async throws {
        let model = MfmPageModel()
        await model.inspect("https://example.com/**:草:** [確認 :ラベル:](https://example.com/:url:) <plain>:plain:</plain> `:code:` $[unknown :unknown:] :smile:")
        #expect(!model.failed)
        #expect(Set(model.emojiCodes) == [":草:", ":ラベル:", ":smile:"])
    }

    @Test("構文がある投稿だけをMFM表示へ渡す", arguments: [
        ("こんにちは 😄 https://example.com @you", false), ("**太字**", true),
        ("$[tada 動き]", true), ("# 見出し", true), ("\\(x^2\\)", true),
        ("[リンク](https://example.com)", true), ("言葉 検索", true)
    ])
    func detection(_ source: String, _ expected: Bool) {
        #expect(MessageContent.requiresMfm(source) == expected)
    }

    @Test("関数名を補完し、コード・plain・URL・エスケープを候補から除く", arguments: [
        ("$", ""), ("$je", "je"), ("本文 $[SP", "sp"), ("`$je", nil),
        ("```\n$je", nil), ("<plain>$je", nil), ("https://example.com/$je", nil), ("\\$je", nil)
    ])
    func completion(_ source: String, _ expected: String?) {
        #expect(MfmInput.query(in: source, caret: source.endIndex)?.text == expected)
    }

    @Test("Web版と共有する全関数の雛形と日本語の説明を同梱する")
    func templates() throws {
        #expect(MfmInput.candidates.count == 24)
        let candidate = try #require(MfmInput.options("FG").first)
        #expect(candidate.template == "$[fg.color=ff0000 ]")
        #expect(candidate.description == "文字色")
    }

    @Test("初回読み込み中のタスク交代でも最新の本文を描画する")
    func loadingReplacement() async throws {
        let model = MfmPageModel()
        let previous = Task { await model.render(payload("**古い本文**")) }
        await Task.yield()
        previous.cancel()
        await model.render(payload("**最新の本文**"))
        await previous.value
        #expect(!model.failed)
        let result = try await model.page.callJavaScript("return document.querySelector('strong')?.textContent;") as? String
        #expect(result == "最新の本文")
    }
}
