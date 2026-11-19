import Foundation

enum ZooError: LocalizedError {
    case invalidConfiguration, invalidResponse, expiredSession, rejected(Int, String), auth(String)

    var errorDescription: String? {
        switch self {
        case .invalidConfiguration: "接続設定を確認してください。サーバーと認証先には HTTPS が必要です。"
        case .invalidResponse: "サーバーの応答を読み取れませんでした。再接続してください。"
        case .expiredSession: "ログインの有効期限が切れました。もう一度ログインしてください。"
        case .rejected(let status, let detail): "操作に失敗しました（\(status)）。\(detail)"
        case .auth(let reason): reason
        }
    }
}
