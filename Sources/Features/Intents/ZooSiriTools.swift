// Siri AIのツールAPIはXcode 27.3以降で提供される。旧SDKでも既存のショートカットをビルドできるようにする。
#if compiler(>=6.4.4)
import AppIntents
import os

// 既存ショートカットの識別子とiOS 26対応を保ち、対応OSだけでSiri AIのツールにもする。
@available(iOS 27.3, *)
extension SendZooChannelMessageIntent: ToolIntent {}

@available(iOS 27.3, *)
@AppIntent(.tool)
struct DraftZooChannelToolIntent: AppIntent {
    static let title: LocalizedStringResource = "Zooのチャンネルの下書きを作成する"
    static let description = IntentDescription("Zooのチャンネルを名前で指定してテキストの下書きを作成し、Zooで編集できるようにします。投稿せずに本文を準備したい場合に使います。")
    static var supportedModes: IntentModes { .foreground }
    static var authenticationPolicy: IntentAuthenticationPolicy { .requiresAuthentication }
    private static let logger = Logger(subsystem: "works.zli.zoo", category: "Siri")

    @Parameter(title: "チャンネル", description: "下書きを作成するZooのチャンネル。名前または#付きの名前で指定します。",
               requestValueDialog: "どのチャンネルに下書きを作成しますか？") var channel: ZooChannelEntity
    @Parameter(title: "メッセージ", description: "下書きに入れるテキストの本文。", default: "") var content: String

    @MainActor func perform() async throws -> some IntentResult {
        Self.logger.notice("Siri AIからのチャンネル下書き作成を開始しました。")
        do {
            let workspace = try await AppSession.shared.prepareForShortcut()
            try await ShortcutActions(workspace: workspace).draft(content, to: channel.channel)
            Self.logger.notice("Siri AIからのチャンネル下書き作成が完了しました。")
            return .result()
        } catch {
            Self.logger.error("Siri AIからのチャンネル下書き作成が終了しました。エラー種別: \(String(describing: type(of: error)), privacy: .public)")
            throw error
        }
    }
}
#endif
