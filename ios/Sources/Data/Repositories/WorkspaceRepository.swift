import Foundation

@MainActor
protocol WorkspaceRepository: AnyObject {
    var state: WorkspaceState { get }
    var onChannelsChanged: (() -> Void)? { get set }
    var onSessionExpired: (() -> Void)? { get set }
    func start()
    func stop()
    func reset()
    func createChannel(name: String, description: String, isPrivate: Bool) async throws -> Channel
    func startDM(_ person: Person) async throws -> Channel
    func join(_ channel: Channel) async throws
    func leave(_ channel: Channel) async throws
    func invite(_ person: Person, to channel: Channel) async throws
    func updateChannel(_ channel: Channel, name: String, description: String) async throws
    func setArchived(_ channel: Channel, archived: Bool) async throws
    func markRead(_ channelID: String, messageID: String) async throws
    func readActivity(_ activity: Activity) async throws
    func updateName(_ name: String) async throws
}
