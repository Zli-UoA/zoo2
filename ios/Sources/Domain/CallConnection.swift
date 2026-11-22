import Foundation

enum CallPeerState: String, Sendable {
    case connecting, connected, reconnecting, failed

    var label: String {
        switch self {
        case .connecting: "音声を接続中"
        case .connected: "通話中"
        case .reconnecting: "音声を再接続中"
        case .failed: "音声の接続に失敗"
        }
    }
}

struct CallPeerDiagnostics: Identifiable, Sendable {
    let id: String
    let state: CallPeerState
    let iceState: String
    let route: String
    let receivedBytes: UInt64
    let sentBytes: UInt64
}

enum CallRetryPolicy {
    static func isTransient(_ error: Error) -> Bool {
        if let error = error as? URLError {
            return [.timedOut, .networkConnectionLost, .notConnectedToInternet,
                    .cannotConnectToHost, .cannotFindHost, .dnsLookupFailed].contains(error.code)
        }
        if case ZooError.rejected(let status, _) = error {
            return [408, 429, 500, 502, 503, 504].contains(status)
        }
        return false
    }
}
