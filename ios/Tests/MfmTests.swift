import Foundation
import SwiftUI
import Testing
import UIKit
@testable import Zoo

@MainActor @Suite("ネイティブMFM")
struct MfmTests {
    private func key(width: CGFloat = 320, dark: Bool = false, size: CGFloat = 17, revealed: Set<Int> = []) -> MfmRenderKey {
        MfmRenderKey(fontSize: size, width: width, dark: dark, emojis: [], mentions: ["you": true], channels: [:],
                     restrictedMentions: false, revealed: revealed, locale: "ja_JP", timeZone: "Asia/Tokyo")
    }
    private func kinds(_ nodes: [MfmNode]) -> [MfmNode.Kind] { nodes.flatMap { [$0.kind] + kinds($0.children) } }

    @Test("見出し・装飾・数式・コード・引用・関数をSwiftで解析する")
    func syntax() {
        let document = MfmDocument("# 見出し\n**太字** <i>斜体</i> ~~取消~~\n$[ruby 漢字 かんじ] $[spin.speed=2s 回転]\n\\(x^2\\)\n```swift\nlet x = 1\n```\n> 引用\n<center>中央</center>")
        let all = Set(kinds(document.nodes))
        #expect(all.isSuperset(of: [.heading, .bold, .italic, .strike, .function, .math, .codeBlock, .quote, .center]))
        #expect(document.animated)
        #expect(document.nodes.first?.children.first?.value == "見出し")
    }

    @Test("コード・plain・数式・URL・未知関数の内側をスタンプの取得対象にしない")
    func symbols() {
        let document = MfmDocument("https://example.com/**:草:** [確認 :ラベル:](https://example.com/:url:) <plain>:plain:</plain> `:code:` \\( :math: \\) $[unknown :unknown:] :smile: :@you:")
        #expect(document.emojiCodes == [":草:", ":ラベル:", ":smile:", ":@you:"])
    }

    @Test("入れ子の関数はコード・plain・リンク内の閉じ括弧で終わらない")
    func nestedFunctions() {
        let document = MfmDocument("$[tada **太字** $[fg.color=abc `]` <plain>]</plain> [リンク](https://example.com/a(b))]]")
        #expect(document.nodes.count == 1)
        #expect(document.nodes[0].kind == .function)
        #expect(kinds(document.nodes).filter { $0 == .function }.count == 2)
        #expect(kinds(document.nodes).contains(.link))
    }

    @Test("日本語・絵文字・エスケープ・壊れた構文を失わない", arguments: [
        "😀日本語\n通常の本文", "閉じない $[spin 本文", "<script>alert(1)</script>",
        "[危険](javascript:alert(1))", "`閉じないコード", "***", "_単語の途中"
    ])
    func literal(_ source: String) {
        let document = MfmDocument(source)
        #expect(!document.nodes.isEmpty)
        #expect(!document.nodes.map(\.plainText).joined().isEmpty)
        let state = MfmNativeState(content: source)
        #expect(!state.prepare(key: key(), images: [:]).isEmpty)
    }

    @Test("全関数・数式・ルビをネイティブのTextとして描画できる")
    func nativeDrawing() throws {
        let functions = MfmFunction.names.sorted().map { name in
            name == "ruby" ? "$[ruby 漢字 かんじ]" : name == "unixtime" ? "$[unixtime 1700000000]" : "$[\(name) 動き]"
        }.joined(separator: " ")
        let state = MfmNativeState(content: "# MFM\n**太字** :smile: @you\n\\(\\frac{1}{2}+x^2\\)\n" + functions)
        let blocks = state.prepare(key: key(), images: [:])
        let content = VStack(alignment: .leading) {
            ForEach(blocks) { block in
                block.text.textRenderer(MfmNativeEffects(time: 0.25, reducedMotion: false))
            }
        }.frame(width: 320).fixedSize(horizontal: false, vertical: true)
        let image = try #require(ImageRenderer(content: content).uiImage)
        #expect(image.size.width == 320 && image.size.height > 30)
        #expect(state.preparationCount == 1)
    }

    @Test("数式を文字列ではなく分数として描き、ルビを本文の幅いっぱいに広げない")
    func nativeMathAndRuby() throws {
        let formula = MfmNativeState(content: "\\[\\frac{1}{2}\\]")
        let math = try #require(formula.prepare(key: key(), images: [:]).first)
        let mathImage = try #require(ImageRenderer(content: math.text).uiImage)
        #expect(mathImage.size.height > 24 && mathImage.size.width < 40)
        let ruby = MfmNativeState(content: "$[ruby 漢字 かんじ]")
        let block = try #require(ruby.prepare(key: key(), images: [:]).first)
        let image = try #require(ImageRenderer(content: block.text).uiImage)
        #expect(image.size.width < 120 && image.size.height > 20)
    }

    @Test("テーマ・文字サイズ・画像・ぼかし解除の変更時だけ描画用データを更新する")
    func invalidation() throws {
        let state = MfmNativeState(content: "$[blur **:草:**]")
        let image = UIGraphicsImageRenderer(size: CGSize(width: 24, height: 24)).image { context in
            UIColor.systemGreen.setFill(); context.fill(CGRect(x: 0, y: 0, width: 24, height: 24))
        }
        var value = key()
        _ = state.prepare(key: value, images: [:])
        _ = state.prepare(key: value, images: [:])
        #expect(state.preparationCount == 1)
        value = key(dark: true, size: 24)
        _ = state.prepare(key: value, images: [:])
        #expect(state.preparationCount == 2)
        let imageKey = MfmRenderKey(fontSize: value.fontSize, width: value.width, dark: true,
                                    emojis: [.init(code: ":草:", image: ObjectIdentifier(image))], mentions: value.mentions,
                                    channels: value.channels, restrictedMentions: false, revealed: [], locale: value.locale, timeZone: value.timeZone)
        _ = state.prepare(key: imageKey, images: [":草:": image])
        _ = state.prepare(key: imageKey, images: [":草:": image])
        #expect(state.preparationCount == 3)
        let blur = try #require(state.document.nodes.first)
        state.revealed.insert(blur.id)
        let row = MessageRenderState()
        let retained = row.mfm(content: state.document.source)
        retained.revealed.insert(blur.id)
        row.cancelLoads()
        #expect(row.mfm(content: state.document.source).revealed == [blur.id])
    }

    @Test("同じ本文・幅・画像では解析と描画用データを再作成しない")
    func preparedCache() {
        let cache = MessageRenderCache()
        let ids = (0..<40).map(String.init)
        let state = cache.state(for: "15").mfm(content: "**本文** \\(x^2\\)")
        _ = state.prepare(key: key(), images: [:])
        cache.retain(visibleIDs: ["15", "16"], orderedIDs: ids)
        cache.retain(visibleIDs: ["16", "17"], orderedIDs: ids)
        let returned = cache.state(for: "15").mfm(content: state.document.source)
        _ = returned.prepare(key: key(), images: [:])
        #expect(returned === state)
        #expect(returned.preparationCount == 1)
        _ = returned.prepare(key: key(width: 240), images: [:])
        #expect(returned.preparationCount == 2)
        #expect(cache.state(for: "15").mfm(content: "編集後の本文") !== state)
    }

    @Test("保持範囲の外へ出たMFM状態を解放する")
    func eviction() {
        let cache = MessageRenderCache()
        weak var old: MfmNativeState?
        do { let value = cache.state(for: "0").mfm(content: "**古い本文**"); old = value }
        #expect(old != nil)
        cache.retain(visibleIDs: ["20"], orderedIDs: (0..<40).map(String.init))
        #expect(old == nil)
    }

    @Test("MFMから通常本文への編集で以前の画像とぼかし解除状態を解放する")
    func editingToPlainText() {
        let row = MessageRenderState()
        weak var previous: MfmNativeState?
        do {
            let value = row.mfmIfNeeded(content: "$[blur 隠す本文]")
            value?.revealed.insert(0)
            previous = value
        }
        #expect(previous != nil)
        #expect(row.mfmIfNeeded(content: "通常の本文") == nil)
        #expect(previous == nil)
        #expect(row.mfmIfNeeded(content: "$[blur 隠す本文]")?.revealed.isEmpty == true)
    }

    @Test("画像ストアの退避中も直近の画像を保持し、URLとアカウントの変更で破棄する")
    func retainedImagesAndAccounts() throws {
        let row = MessageRenderState()
        let content = "**:草:**"
        let state = row.mfm(content: content, scope: "account-a")
        let image = UIGraphicsImageRenderer(size: CGSize(width: 24, height: 24)).image { _ in }
        let url = try #require(URL(string: "https://example.com/stamp.png"))
        #expect(state.retainImages(urls: [":草:": url], available: [":草:": image])[":草:"] === image)
        #expect(state.retainImages(urls: [":草:": url], available: [:])[":草:"] === image)
        let changed = try #require(URL(string: "https://example.com/updated.png"))
        #expect(state.retainImages(urls: [":草:": changed], available: [:]).isEmpty)
        state.revealed.insert(0)
        let other = row.mfm(content: content, scope: "account-b")
        #expect(other !== state && other.revealed.isEmpty)
        #expect(other.retainImages(urls: [":草:": url], available: [:]).isEmpty)
    }

    @Test("数式・ルビの同じ画像を共有し、投稿全体の実ピクセル予算を超えない")
    func imageBudget() {
        let repeated = MfmNativeState(content: "$[x4 " + String(repeating: "\\(x\\) $[ruby 漢字 かんじ] ", count: 500) + "]")
        _ = repeated.prepare(key: key(), images: [:])
        #expect(repeated.rasterImageCount == 2)
        #expect(repeated.rasterPixels <= 524_288)
        let distinct = MfmNativeState(content: (0..<150).map { "\\(x_{\($0)}\\) $[ruby 漢字 かんじ\($0)]" }.joined(separator: " "))
        _ = distinct.prepare(key: key(), images: [:])
        #expect(distinct.rasterImageCount <= 64 && distinct.rasterPixels <= 524_288)
        let tall = MfmNativeState(content: "$[x4 \\[\\begin{matrix}" + Array(repeating: "11111", count: 30).joined(separator: "\\\\") + "\\end{matrix}\\]]")
        _ = tall.prepare(key: key(), images: [:])
        #expect(tall.rasterPixels <= 524_288)
    }

    @Test("絵文字・数式・ルビの画像ラベルへリンクの当たり判定を保持する")
    func imageLinks() throws {
        let state = MfmNativeState(content: "[:草:](https://example.com/emoji) [\\(x^2\\)](https://example.com/math) [$[ruby 漢字 かんじ]](https://example.com/ruby)")
        let image = UIGraphicsImageRenderer(size: CGSize(width: 24, height: 24)).image { context in
            UIColor.systemGreen.setFill(); context.fill(CGRect(x: 0, y: 0, width: 24, height: 24))
        }
        let block = try #require(state.prepare(key: key(), images: [":草:": image]).first)
        #expect(Set(block.imageLinks.map { $0.url.path }) == ["/emoji", "/math", "/ruby"])
        let content = block.text.textRenderer(MfmNativeEffects(time: 0, reducedMotion: true, interactions: block.interactions))
        let snapshot = try #require(ImageRenderer(content: content).uiImage)
        var paths: Set<String> = []
        for x in stride(from: 0.0, through: snapshot.size.width, by: 2) {
            for y in stride(from: 0.0, through: snapshot.size.height, by: 2) {
                if let url = block.interactions.url(at: CGPoint(x: x, y: y)) { paths.insert(url.path) }
            }
        }
        #expect(paths == ["/emoji", "/math", "/ruby"])
    }

    @Test("ルビの中の複数リンクと移動した文字リンクの描画位置を保持する")
    func nestedLinks() throws {
        let source = "$[ruby [漢](https://example.com/first)[字](https://example.com/second) かんじ] $[position.x=2 [a](https://example.com/shifted)]"
        let block = try #require(MfmNativeState(content: source).prepare(key: key(), images: [:]).first)
        #expect(block.accessibilityLabel == "漢字（かんじ） a")
        #expect(Set(block.imageLinks.map { $0.url.path }) == ["/first", "/second", "/shifted"])
        let view = block.text.textRenderer(MfmNativeEffects(time: 0, reducedMotion: true, interactions: block.interactions)).frame(width: 320)
        _ = try #require(ImageRenderer(content: view).uiImage)
        #expect(Set(block.interactions.snapshot().map { $0.url.path }) == ["/first", "/second", "/shifted"])
        let first = try #require(block.interactions.snapshot().first { $0.url.path == "/first" })
        let second = try #require(block.interactions.snapshot().first { $0.url.path == "/second" })
        #expect(first.bounds.midX < second.bounds.midX)
        #expect(block.interactions.url(at: CGPoint(x: first.bounds.midX, y: first.bounds.midY))?.path == "/first")
        #expect(block.interactions.url(at: CGPoint(x: second.bounds.midX, y: second.bounds.midY))?.path == "/second")
    }

    @Test("掛け算や空白に接した区切り記号を斜体にしない", arguments: ["5 * 3 * 4", "* 前後に空白 *", "_ 前後に空白 _"])
    func whitespaceDelimiters(_ source: String) {
        let document = MfmDocument(source)
        #expect(!kinds(document.nodes).contains(.italic))
        #expect(document.nodes.map(\.plainText).joined() == source)
        #expect(MfmNativeState(content: source).prepare(key: key(), images: [:]).first?.accessibilityLabel == source)
        #expect(kinds(MfmDocument("*斜体*").nodes).contains(.italic))
    }

    @Test("既存のURL検出器が返した大文字スキームをリンクとして保つ")
    func uppercaseLinks() throws {
        let block = try #require(MfmNativeState(content: "**資料** HTTPS://example.com/path").prepare(key: key(), images: [:]).first)
        #expect(block.imageLinks.contains { $0.url.host == "example.com" && $0.url.path == "/path" })
        for raw in ["HTTPS://example.com/search?user=@alice", "hTtP://example.com/search?user=@alice"] {
            let content = "**資料** " + raw
            let link = try #require(MfmDocument(content).nodes.first { $0.kind == .link })
            #expect(link.argument == raw)
            let rendered = try #require(MfmNativeState(content: content).prepare(key: key(), images: [:]).first)
            let url = try #require(rendered.imageLinks.first { $0.url.host == "example.com" }?.url)
            #expect(URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?.first?.value == "@alice")
        }
        for (raw, destination) in [("HTTPS://example.com/pathを見てください", "HTTPS://example.com/path"),
                                   ("HTTPS://example.com/search?user=＠alice", "HTTPS://example.com/search?user=")] {
            let nodes = MfmDocument("**資料** " + raw).nodes
            #expect(nodes.first { $0.kind == .link }?.argument == destination)
            #expect(nodes.last?.plainText == String(raw.dropFirst(destination.count)))
        }
    }

    @Test("登録済み絵文字の画像が未取得なら標準絵文字に置き換えない")
    func registeredEmojiPlaceholder() throws {
        let state = MfmNativeState(content: "**:smile:**")
        let value = MfmRenderKey(fontSize: 17, width: 320, dark: false,
            emojis: [.init(code: ":smile:", image: nil, registered: true)], mentions: [:], channels: [:],
            restrictedMentions: false, revealed: [], locale: "ja_JP", timeZone: "Asia/Tokyo")
        #expect(try #require(state.prepare(key: value, images: [:]).first).accessibilityLabel == ":smile:")
        #expect(try #require(state.prepare(key: key(), images: [:]).first).accessibilityLabel != ":smile:")
    }

    @Test("解除前のぼかしを読み上げラベルと操作名に含めない")
    func hiddenBlurAccessibility() throws {
        let state = MfmNativeState(content: "前 $[blur **結末** :smile: $[ruby 漢字 かんじ]] 後")
        let hidden = try #require(state.prepare(key: key(), images: [:]).first)
        #expect(hidden.accessibilityLabel == "前 ぼかしあり 後")
        #expect(hidden.imageLinks.map(\.label) == ["ぼかしを解除"])
        let blur = try #require(state.document.nodes.first { $0.kind == .function })
        let revealed = try #require(state.prepare(key: key(revealed: [blur.id]), images: [:]).first)
        #expect(revealed.accessibilityLabel.contains("結末"))
        let ruby = MfmNativeState(content: "$[ruby $[blur 結末] けつまつ]")
        #expect(try #require(ruby.prepare(key: key(), images: [:]).first).accessibilityLabel == "ぼかしあり（けつまつ）")
    }

    @Test("スレッドと投稿のリンク先を別々に保持する")
    func distinctThreadTarget() throws {
        let channel = "11111111-1111-4111-8111-111111111111", parent = "22222222-2222-4222-8222-222222222222", reply = "33333333-3333-4333-8333-333333333333"
        let url = try #require(URL(string: "https://zli.chat/channels/\(channel)?thread=\(parent)&message=\(reply)"))
        let target = try #require(MessageLink.target(url, serverURL: "https://zli.chat"))
        #expect(target.channelID == channel && target.threadID == parent && target.messageID == reply)
    }

    @Test("チャンネル・投稿・スレッド・返信のURLを作成して同じ移動先へ戻せる", arguments: [0, 1, 2, 3])
    func generatedLinks(kind: Int) throws {
        let channel = "11111111-1111-4111-8111-111111111111", parent = "22222222-2222-4222-8222-222222222222", reply = "33333333-3333-4333-8333-333333333333"
        let message = kind == 1 ? parent : kind == 3 ? reply : nil
        let thread = kind >= 2 ? parent : nil
        let url = try #require(MessageLink.url(serverURL: "https://zli.chat/", channelID: channel, messageID: message, threadID: thread))
        #expect(MessageLink.target(url, serverURL: "https://zli.chat") == MessageLink.Target(channelID: channel, messageID: message, threadID: thread))
        #expect(url.host == "zli.chat" && !url.absoluteString.contains("//channels"))
        #expect(MessageLink.url(serverURL: "https://zli.chat", channelID: "不正") == nil)
        #expect(MessageLink.url(serverURL: "javascript:alert(1)", channelID: channel) == nil)
    }

    @Test("IME変換中は補完キーを無効にし、確定後に戻す")
    func compositionPrecedesCompletion() throws {
        let scene = try #require(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.first)
        let previous = scene.keyWindow
        let window = UIWindow(windowScene: scene)
        let controller = UIViewController()
        window.rootViewController = controller
        window.makeKeyAndVisible()
        let input = MfmInputTextView(frame: CGRect(x: 0, y: 0, width: 200, height: 100))
        input.completionEnabled = true
        controller.view.addSubview(input)
        defer { input.resignFirstResponder(); window.isHidden = true; previous?.makeKeyAndVisible() }
        #expect(input.becomeFirstResponder())
        input.setMarkedText("かな", selectedRange: NSRange(location: 2, length: 0))
        let action = #selector(MfmInputTextView.completeMfmKey(_:))
        let escape = try #require(input.keyCommands?.first { $0.input == UIKeyCommand.inputEscape })
        let enter = try #require(input.keyCommands?.first { $0.input == "\r" })
        var received: [String] = []
        input.handle = { received.append($0) }
        input.completeMfmKey(escape)
        input.completeMfmKey(enter)
        #expect(received.isEmpty)
        #expect(!input.canPerformAction(action, withSender: nil))
        input.unmarkText()
        #expect(input.keyCommands?.contains { $0.action == action } == true)
        #expect(input.canPerformAction(action, withSender: nil))
        input.completeMfmKey(escape)
        input.completeMfmKey(enter)
        #expect(received == [UIKeyCommand.inputEscape, "\r"])
    }

    @Test("日本語と絵文字の後の補完範囲を文字列の保存形式によらず保持する")
    func unicodeCompletionRange() throws {
        let text = "日本語😀\n本文 $je"
        let foreign = NSMutableString(string: text) as String
        let nativeQuery = try #require(MfmInput.query(in: text, caret: text.endIndex))
        let foreignQuery = try #require(MfmInput.query(in: foreign, caret: foreign.endIndex))
        #expect(nativeQuery.utf16Range == NSRange(location: "日本語😀\n本文 ".utf16.count, length: 3))
        #expect(nativeQuery.utf16Range == foreignQuery.utf16Range)
        #expect(nativeQuery.text == "je" && foreignQuery.text == "je")
    }

    @Test("装飾中の句点付きメンションにも許可条件を適用する")
    func mentions() {
        #expect(MessageContent.mfmMentions(in: "**@you.** $[spin @other]", resolve: { $0 == "you" ? true : nil }) == ["you": true])
    }

    @Test("同じ接続先のチャンネルリンクだけをアプリ内で開く")
    func routing() throws {
        let channel = "11111111-1111-4111-8111-111111111111"
        let message = "22222222-2222-4222-8222-222222222222"
        let url = try #require(URL(string: "https://zli.chat/channels/\(channel)?thread=\(message)"))
        let target = try #require(MessageLink.target(url, serverURL: "https://zli.chat:443"))
        #expect(target.channelID == channel && target.threadID == message && target.messageID == nil)
        #expect(MessageLink.target(url, serverURL: "https://other.invalid") == nil)
    }

    @Test("関数名を補完し、保護領域を候補から除く", arguments: [
        ("$", ""), ("$je", "je"), ("本文 $[SP", "sp"), ("`$je", nil),
        ("```\n$je", nil), ("<plain>$je", nil), ("https://example.com/$je", nil), ("\\$je", nil)
    ])
    func completion(_ source: String, _ expected: String?) {
        #expect(MfmInput.query(in: source, caret: source.endIndex)?.text == expected)
    }

    @Test("関数一覧・時間と数値の上限を全クライアントと揃える")
    func limits() throws {
        #expect(MfmInput.candidates.count == MfmFunction.names.count)
        let node = try #require(MfmDocument("$[spin.speed=1ms,delay=-999999s,x=Infinity 回転]").nodes.first)
        let function = MfmFunction(node)
        #expect(function.seconds("speed", fallback: 1) == 0.5)
        #expect(function.seconds("delay", fallback: 0, delay: true) == -86400)
        #expect(function.number("x", fallback: 1, range: -5...5) == 1)
    }

    @Test("深い入れ子・閉じない構文・長文でも解析量を制限する")
    func boundedParsing() {
        let deep = String(repeating: "$[spin ", count: 100) + "本文" + String(repeating: "]", count: 100)
        #expect(!MfmDocument(deep).nodes.isEmpty)
        #expect(!MfmDocument(String(repeating: "$[spin ** [", count: 3_000)).nodes.isEmpty)
        #expect(!MfmDocument(String(repeating: ">", count: 40_000)).nodes.isEmpty)
        let quotes = String(repeating: "> " + String(repeating: "[", count: 1_990) + "\n\n", count: 20)
        let elapsed = ContinuousClock().measure {
            #expect(!MfmDocument(quotes).nodes.isEmpty)
        }
        #expect(elapsed < .seconds(3))
        let text = String(repeating: "😀日本語\n", count: 6_000)
        #expect(MfmDocument(text).nodes.map(\.plainText).joined() == text)
    }
}
