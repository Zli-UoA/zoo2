#if DEBUG
import Foundation

// UI テストは本番の認証・通知・投稿に触れず、実際の SwiftUI 画面を操作する。
@MainActor
final class PreviewCallMedia: CallMedia {
    func start(sessionID: String, room: CallRoom, signal: @escaping @MainActor (String, CallSignal) -> Void,
               failed: @escaping @MainActor () -> Void) throws {}
    func handle(_ event: CallEvent) async throws {}
    func mute(_ muted: Bool) {}
    func speaker(_ enabled: Bool) throws {}
    func close() {}
}
#endif
