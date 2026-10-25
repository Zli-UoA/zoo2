import Foundation

@MainActor
final class DemoWorkspaceRepository: WorkspaceRepository {
    let state = WorkspaceState()
    var onChannelsChanged: (() -> Void)?
    var onSessionExpired: (() -> Void)?
    private(set) var messages: [Message] = []
    private(set) var reactions: [Reaction] = []

    init() {
        state.me = Person(id: "demo-me", user_name: "you", display_name: "あなた")
        state.people = [state.me!, Person(id: "demo-aki", user_name: "aki", display_name: "Aki"),
                  Person(id: "demo-haru", user_name: "haru", display_name: "Haru"),
                  Person(id: "demo-bot", user_name: "github", display_name: "GitHub", is_bot: true)]
        state.channels = [
            Channel(id: "general", channel_name: "general", description: "Zli のみんなが集まる場所", created_user_id: "demo-me", is_private: false, is_dm: false),
            Channel(id: "dev", channel_name: "開発", description: "作っているもの、気になった技術の話", created_user_id: "demo-aki", is_private: false, is_dm: false),
            Channel(id: "random", channel_name: "雑談", description: "なんでも気軽に。", created_user_id: "demo-haru", is_private: false, is_dm: false),
            Channel(id: "events", channel_name: "イベント", description: "次の勉強会を計画しよう", created_user_id: "demo-aki", is_private: false, is_dm: false),
            Channel(id: "dm", channel_name: "DM", created_user_id: "demo-me", is_private: true, is_dm: true)
        ]
        state.memberships = state.channels.filter { $0.id != "events" }.flatMap { channel in
            [Membership(user_id: "demo-me", channel_id: channel.id), Membership(user_id: "demo-aki", channel_id: channel.id)]
        }
        let now = Date()
        func time(_ minutes: Double) -> String { ISO8601DateFormatter().string(from: now.addingTimeInterval(-minutes * 60)) }
        messages = [
            Message(id: "welcome", user_id: "demo-aki", channel_id: "general", content: "Zli の新しい居場所へようこそ 👋\n作っているものや、ちょっとした発見をシェアしよう。", created_at: time(60)),
            Message(id: "meetup", user_id: "demo-haru", channel_id: "general", content: "今週のもくもく会、金曜の夕方はどうですか？\n途中参加も歓迎です ☕️", created_at: time(35)),
            Message(id: "answer", user_id: "demo-me", channel_id: "general", content: "参加します！iOS アプリの続きを進めたいです。", created_at: time(30)),
            Message(id: "thread", user_id: "demo-aki", channel_id: "general", content: "いいね！18時からにしよう 🙌", created_at: time(25), parent_id: "meetup"),
            Message(id: "demo-dev", user_id: "demo-aki", channel_id: "dev", content: "小さく作って、動かして、改善する。\n今日の進捗をここに残していこう。", created_at: time(20)),
            Message(id: "demo-dm", user_id: "demo-aki", channel_id: "dm", content: "もくもく会でアプリ見せてね！", created_at: time(10))
        ]
        reactions = [Reaction(id: "r1", message_id: "welcome", user_id: "demo-haru", emoji: "🎉"),
                     Reaction(id: "r2", message_id: "welcome", user_id: "demo-aki", emoji: "🎉"),
                     Reaction(id: "r3", message_id: "meetup", user_id: "demo-me", emoji: "👍")]
        state.activities = [Activity(id: "n1", actor_id: "demo-aki", channel_id: "general", message_id: "thread", notification_type: "reply", is_read: false, created_at: time(25)),
                      Activity(id: "n2", actor_id: "demo-aki", channel_id: "dm", message_id: "demo-dm", notification_type: "dm", is_read: false, created_at: time(10))]
        state.unread = ["dev", "dm"]
        state.channelPhase = .ready
    }

    func start() {}
    func stop() {}
    func reset() { state.reset(); messages = []; reactions = [] }

    func makeMessages(channelID: String) -> DemoMessageRepository {
        DemoMessageRepository(channelID: channelID, userID: state.me?.id ?? "", messages: messages, reactions: reactions)
    }

    func createChannel(name: String, description: String, isPrivate: Bool) async throws -> Channel {
        guard let me = state.me else { throw ZooError.expiredSession }
        let channel = Channel(id: UUID().uuidString, channel_name: name, description: description,
                              created_user_id: me.id, is_private: isPrivate, is_dm: false)
        state.channels.append(channel)
        state.memberships.append(Membership(user_id: me.id, channel_id: channel.id))
        onChannelsChanged?()
        return channel
    }

    func startDM(_ person: Person) async throws -> Channel {
        guard let me = state.me else { throw ZooError.expiredSession }
        let channel = Channel(id: UUID().uuidString, channel_name: "DM", created_user_id: me.id, is_private: true, is_dm: true)
        state.channels.append(channel)
        state.memberships += [Membership(user_id: me.id, channel_id: channel.id), Membership(user_id: person.id, channel_id: channel.id)]
        onChannelsChanged?()
        return channel
    }

    func join(_ channel: Channel) async throws {
        guard let me = state.me else { throw ZooError.expiredSession }
        state.memberships.append(Membership(user_id: me.id, channel_id: channel.id))
    }

    func leave(_ channel: Channel) async throws {
        state.memberships.removeAll { $0.channel_id == channel.id && $0.user_id == state.me?.id }
        if channel.is_private { state.channels.removeAll { $0.id == channel.id }; onChannelsChanged?() }
    }

    func invite(_ person: Person, to channel: Channel) async throws {
        state.memberships.append(Membership(user_id: person.id, channel_id: channel.id))
    }

    func updateChannel(_ channel: Channel, name: String, description: String) async throws {
        if let index = state.channels.firstIndex(where: { $0.id == channel.id }) {
            state.channels[index].channel_name = name
            state.channels[index].description = description
        }
    }

    func setArchived(_ channel: Channel, archived: Bool) async throws {
        if let index = state.channels.firstIndex(where: { $0.id == channel.id }) {
            state.channels[index].archive_at = archived ? ZooDate.now() : nil
        }
    }

    func markRead(_ channelID: String, messageID: String) async throws { state.unread.remove(channelID) }

    func readActivity(_ activity: Activity) async throws {
        if let index = state.activities.firstIndex(where: { $0.id == activity.id }) { state.activities[index].is_read = true }
    }

    func updateName(_ name: String) async throws { state.me?.display_name = name }
}
