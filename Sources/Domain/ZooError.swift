import Foundation

enum ZooError: LocalizedError {
    case invalidConfiguration, invalidResponse, expiredSession, rejected(Int, String), auth(String)
    /// 参加行があるスレッドへ、未参加・退出済みのまま返信したときの 403（サーバーの `create_message_impl`）。
    /// 参加すれば同じ返信を送れる。投稿権限がないなど、ほかの 403 は含まない（`rejected`）。
    case threadJoinRequired(String)

    static let threadJoinRequiredCode = "THREAD_JOIN_REQUIRED"
    /// `code` を返す前のサーバーが、参加不足の 403 に付ける本文。
    static let threadJoinRequiredMessage = "Join the thread before replying"

    /// `code` があれば `code` だけで決め、`code` のない応答（コードを返す前のサーバー）だけ、本文で決める。
    static func requiresThreadJoin(code: String?, message: String) -> Bool {
        if let code { return code == threadJoinRequiredCode }
        return message.contains(threadJoinRequiredMessage)
    }

    var errorDescription: String? {
        switch self {
        case .invalidConfiguration: "接続設定を確認してください。サーバーと認証先には HTTPS が必要です。"
        case .invalidResponse: "サーバーの応答を読み取れませんでした。再接続してください。"
        case .expiredSession: "ログインの有効期限が切れました。もう一度ログインしてください。"
        case .rejected(let status, let detail): "操作に失敗しました（\(status)）。\(detail)"
        case .threadJoinRequired(let detail): "操作に失敗しました（403）。\(detail)"
        case .auth(let reason): reason
        }
    }
}
