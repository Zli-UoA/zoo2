import Foundation

@MainActor
protocol MessageRepository: AnyObject {
    var state: ConversationState { get }
    var onAccessDenied: (() -> Void)? { get set }
    var onSessionExpired: (() -> Void)? { get set }
    func start()
    func stop()
    func reset()
    func send(id: String, text: String, parentID: String?) async throws
    func edit(_ message: Message, content: String) async throws
    func delete(_ message: Message) async throws
    func react(_ message: Message, emoji: String) async throws
}
