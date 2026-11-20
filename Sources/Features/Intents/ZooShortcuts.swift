import AppIntents
import CoreSpotlight
import os

struct ZooRecipientEntity: AppEntity {
    static let typeDisplayRepresentation: TypeDisplayRepresentation = "Zooのメンバー"
    static let defaultQuery = ZooRecipientQuery()
    let id: String
    let personID: String
    let name: String
    let username: String

    init(_ recipient: ShortcutRecipient) {
        id = recipient.id; personID = recipient.personID; name = recipient.name; username = recipient.username
    }

    var recipient: ShortcutRecipient { .init(id: id, personID: personID, name: name, username: username) }
    var displayRepresentation: DisplayRepresentation { .init(title: "\(name)", subtitle: "@\(username)") }
}

struct ZooRecipientQuery: EntityStringQuery {
    @MainActor private func recipients() async throws -> [ZooRecipientEntity] {
        let workspace = try await AppSession.shared.prepareForShortcut()
        return try await ShortcutActions(workspace: workspace).recipients().map(ZooRecipientEntity.init)
    }

    func entities(for identifiers: [String]) async throws -> [ZooRecipientEntity] {
        let ids = Set(identifiers)
        return try await recipients().filter { ids.contains($0.id) }
    }

    func entities(matching string: String) async throws -> [ZooRecipientEntity] {
        try await recipients().filter { $0.recipient.matches(string) }
    }

    func suggestedEntities() async throws -> [ZooRecipientEntity] {
        do { return try await recipients() }
        catch {
            if await AppSession.shared.userID == nil { return [] }
            throw error
        }
    }
}

struct SendZooMessageIntent: AppIntent {
    static let title: LocalizedStringResource = "Zooでメッセージを送る"
    static let description = IntentDescription("ZooのメンバーとのDMにメッセージを送信します。")
    static var supportedModes: IntentModes { .foreground }
    static var authenticationPolicy: IntentAuthenticationPolicy { .requiresAuthentication }

    @Parameter(title: "宛先", requestValueDialog: "誰に送りますか？") var recipient: ZooRecipientEntity
    @Parameter(title: "メッセージ", requestValueDialog: "どんなメッセージを送りますか？") var content: String

    static var parameterSummary: some ParameterSummary { Summary("\(\.$recipient)に\(\.$content)を送る") }

    @MainActor func perform() async throws -> some IntentResult & ProvidesDialog {
        let workspace = try await AppSession.shared.prepareForShortcut()
        let actions = ShortcutActions(workspace: workspace)
        try ShortcutActions.validateMessage(content)
        let person = try await actions.resolve(recipient.recipient)
        try await requestConfirmation(dialog: "\(person.name)さんに「\(content)」を送りますか？")
        try await actions.send(content, to: recipient.recipient)
        return .result(dialog: "\(person.name)さんに送信しました。")
    }
}

struct ZooChannelEntity: IndexedEntity {
    static let typeDisplayRepresentation: TypeDisplayRepresentation = "Zooのチャンネル"
    static let defaultQuery = ZooChannelQuery()
    let id: String
    let channelID: String
    let name: String

    init(_ channel: ShortcutChannel) {
        id = channel.id; channelID = channel.channelID; name = channel.name
    }

    var channel: ShortcutChannel { .init(id: id, channelID: channelID, name: name) }
    var displayRepresentation: DisplayRepresentation { .init(title: "#\(name)") }
}

struct ZooChannelQuery: EntityStringQuery {
    private static let logger = Logger(subsystem: "works.zli.zoo", category: "Siri")
    private let prepare: @MainActor @Sendable () async throws -> WorkspaceStore

    init() {
        self.init { try await AppSession.shared.prepareForShortcut() }
    }

    init(prepare: @escaping @MainActor @Sendable () async throws -> WorkspaceStore) {
        self.prepare = prepare
    }

    @MainActor private func channels() async throws -> [ShortcutChannel] {
        do {
            let workspace = try await prepare()
            let channels = try await ShortcutActions(workspace: workspace).channels()
            Self.logger.notice("チャンネル検索の最新一覧: \(channels.count)件")
            return channels
        } catch {
            Self.logger.error("チャンネル検索の一覧取得に失敗: \(String(describing: type(of: error)), privacy: .public)")
            throw error
        }
    }

    func entities(for identifiers: [String]) async throws -> [ZooChannelEntity] {
        Self.logger.notice("チャンネルのID検索: \(identifiers.count)件")
        let ids = Set(identifiers)
        let result = try await channels().filter { ids.contains($0.id) }.map(ZooChannelEntity.init)
        Self.logger.notice("チャンネルのID検索結果: \(result.count)件、未解決ID: \(ids.subtracting(result.map(\.id)).count)件")
        return result
    }

    func entities(matching string: String) async throws -> [ZooChannelEntity] {
        Self.logger.notice("チャンネルの名前検索を開始しました。検索語: \(string, privacy: .private(mask: .hash))")
        let result = try await ShortcutChannel.matching(identifier: nil, name: string, in: channels()).map(ZooChannelEntity.init)
        Self.logger.notice("チャンネルの名前検索結果: \(result.count)件")
        return result
    }

    func suggestedEntities() async throws -> [ZooChannelEntity] {
        Self.logger.notice("チャンネルの候補一覧を要求されました。")
        do { return try await channels().map(ZooChannelEntity.init) }
        catch {
            if await AppSession.shared.userID == nil { return [] }
            throw error
        }
    }
}

struct SendZooChannelMessageIntent: AppIntent {
    private static let logger = Logger(subsystem: "works.zli.zoo", category: "Siri")
    static let title: LocalizedStringResource = "Zooのチャンネルに投稿する"
    static let description = IntentDescription("Zooのチャンネルにメッセージを投稿します。")
    static var supportedModes: IntentModes { .foreground }
    static var authenticationPolicy: IntentAuthenticationPolicy { .requiresAuthentication }

    @Parameter(title: "チャンネル", requestValueDialog: "どのチャンネルに投稿しますか？") var channel: ZooChannelEntity
    @Parameter(title: "メッセージ", requestValueDialog: "どんなメッセージを投稿しますか？") var content: String

    static var parameterSummary: some ParameterSummary { Summary("\(\.$channel)に\(\.$content)を投稿する") }

    @MainActor func perform() async throws -> some IntentResult & ProvidesDialog {
        Self.logger.notice("チャンネル投稿の実行を開始しました。")
        let workspace = try await AppSession.shared.prepareForShortcut()
        let actions = ShortcutActions(workspace: workspace)
        try ShortcutActions.validateMessage(content)
        let target = try await actions.resolve(channel.channel)
        Self.logger.notice("チャンネル投稿の送信先を解決し、確認を要求します。")
        try await requestConfirmation(dialog: "#\(target.channel_name)に「\(content)」を投稿しますか？")
        Self.logger.notice("チャンネル投稿の確認が完了しました。")
        try await actions.send(content, to: channel.channel)
        return .result(dialog: "#\(target.channel_name)に投稿しました。")
    }
}

struct StartZooCallIntent: AppIntent {
    static let title: LocalizedStringResource = "Zooで電話をかける"
    static let description = IntentDescription("ZooのメンバーとのDMで音声通話を始めます。")
    static var supportedModes: IntentModes { .foreground }
    static var authenticationPolicy: IntentAuthenticationPolicy { .requiresAuthentication }

    @Parameter(title: "相手", requestValueDialog: "誰に電話をかけますか？") var recipient: ZooRecipientEntity
    static var parameterSummary: some ParameterSummary { Summary("\(\.$recipient)に電話をかける") }

    @MainActor func perform() async throws -> some IntentResult & ProvidesDialog {
        let workspace = try await AppSession.shared.prepareForShortcut()
        let actions = ShortcutActions(workspace: workspace)
        let person = try await actions.resolve(recipient.recipient)
        try await requestConfirmation(dialog: "\(person.name)さんに電話をかけますか？")
        try await actions.call(recipient.recipient)
        return .result(dialog: "\(person.name)さんとの通話を開始しました。")
    }
}

struct ZooShortcuts: AppShortcutsProvider {
    static var appShortcuts: [AppShortcut] {
        AppShortcut(intent: SendZooMessageIntent(), phrases: [
            "\(.applicationName)で\(\.$recipient)にメッセージを送って",
            "\(.applicationName)でメッセージを送って"
        ], shortTitle: "メッセージを送る", systemImageName: "paperplane")
        AppShortcut(intent: SendZooChannelMessageIntent(), phrases: [
            "\(.applicationName)の\(\.$channel)に投稿して",
            "\(.applicationName)のチャンネルに投稿して"
        ], shortTitle: "チャンネルに投稿", systemImageName: "number")
        AppShortcut(intent: StartZooCallIntent(), phrases: [
            "\(.applicationName)で\(\.$recipient)に電話をかけて",
            "\(.applicationName)で電話をかけて"
        ], shortTitle: "電話をかける", systemImageName: "phone")
    }
}
