import AppIntents
import CoreSpotlight
import os

struct ZooRecipientEntity: IndexedEntity {
    static let typeDisplayRepresentation: TypeDisplayRepresentation = "Zooのメンバー"
    static let defaultQuery = ZooRecipientQuery()
    let id: String
    let personID: String
    @Property(title: "表示名") var name: String
    @Property(title: "ユーザー名") var username: String

    init(_ recipient: ShortcutRecipient) {
        id = recipient.id; personID = recipient.personID; name = recipient.name; username = recipient.username
    }

    var recipient: ShortcutRecipient { .init(id: id, personID: personID, name: name, username: username) }
    var displayRepresentation: DisplayRepresentation { .init(title: "\(name)", subtitle: "@\(username)") }
}

struct ZooRecipientQuery: EntityStringQuery, EnumerableEntityQuery {
    private let prepare: @MainActor @Sendable () async throws -> WorkspaceStore

    init() { self.init { try await AppSession.shared.prepareForShortcut() } }
    init(prepare: @escaping @MainActor @Sendable () async throws -> WorkspaceStore) { self.prepare = prepare }

    @MainActor private func recipients() async throws -> [ZooRecipientEntity] {
        let workspace = try await prepare()
        return try await ShortcutActions(workspace: workspace).recipients().map(ZooRecipientEntity.init)
    }

    func entities(for identifiers: [String]) async throws -> [ZooRecipientEntity] {
        let ids = Set(identifiers)
        return try await recipients().filter { ids.contains($0.id) }
    }

    func entities(matching string: String) async throws -> [ZooRecipientEntity] {
        let candidates = try await recipients()
        let exact = candidates.filter { $0.recipient.exactlyMatches(string) }
        return exact.isEmpty ? candidates.filter { $0.recipient.matches(string) } : exact
    }

    func allEntities() async throws -> [ZooRecipientEntity] { try await recipients() }

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
    static let description = IntentDescription("Zooのメンバーを表示名またはユーザー名で指定し、その人とのDMにテキストを送信します。")
    // Siriの送信確認を終えるまでアプリを前面に出さない。
    static var supportedModes: IntentModes { .foreground(.deferred) }
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
    @Property(title: "チャンネル名") var name: String

    init(_ channel: ShortcutChannel) {
        id = channel.id; channelID = channel.channelID; name = channel.name
    }

    var channel: ShortcutChannel { .init(id: id, channelID: channelID, name: name) }
    var displayRepresentation: DisplayRepresentation { .init(title: "#\(name)") }
}

struct ZooChannelQuery: EntityStringQuery, EnumerableEntityQuery {
    private static let logger = Logger(subsystem: "works.zli.zoo", category: "Siri")
    private let prepare: @MainActor @Sendable () async throws -> WorkspaceStore

    init() {
        self.init { try await AppSession.shared.prepareForShortcut() }
    }

    init(prepare: @escaping @MainActor @Sendable () async throws -> WorkspaceStore) {
        self.prepare = prepare
    }

    @MainActor private func channels(suggested: Bool = false) async throws -> [ShortcutChannel] {
        do {
            let workspace = try await prepare()
            let actions = ShortcutActions(workspace: workspace)
            let channels = try await suggested ? actions.suggestedChannels() : actions.channels()
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
        do { return try await channels(suggested: true).map(ZooChannelEntity.init) }
        catch {
            if await AppSession.shared.userID == nil { return [] }
            throw error
        }
    }

    func allEntities() async throws -> [ZooChannelEntity] {
        try await channels().map(ZooChannelEntity.init)
    }
}

struct SendZooChannelMessageIntent: AppIntent {
    private static let logger = Logger(subsystem: "works.zli.zoo", category: "Siri")
    static let title: LocalizedStringResource = "Zooのチャンネルに投稿する"
    static let description = IntentDescription("Zooのチャンネルを名前で指定し、テキストを投稿します。DMではなくチャンネルへの投稿に使います。")
    // Siriの投稿確認を終えるまでアプリを前面に出さない。
    static var supportedModes: IntentModes { .foreground(.deferred) }
    static var authenticationPolicy: IntentAuthenticationPolicy { .requiresAuthentication }

    @Parameter(title: "チャンネル", description: "投稿先のZooのチャンネル。名前または#付きの名前で指定します。",
               requestValueDialog: "どのチャンネルに投稿しますか？") var channel: ZooChannelEntity
    @Parameter(title: "メッセージ", description: "チャンネルに投稿するテキストの本文。",
               requestValueDialog: "どんなメッセージを投稿しますか？") var content: String

    static var parameterSummary: some ParameterSummary { Summary("\(\.$channel)に\(\.$content)を投稿する") }

    @MainActor func perform() async throws -> some IntentResult & ProvidesDialog {
        Self.logger.notice("チャンネル投稿の実行を開始しました。")
        let workspace = try await AppSession.shared.prepareForShortcut()
        let actions = ShortcutActions(workspace: workspace)
        try ShortcutActions.validateMessage(content)
        let target = try await actions.resolve(channel.channel)
        Self.logger.notice("チャンネル投稿の送信先を解決し、確認を要求します。")
        try await requestConfirmation(dialog: "Zooの#\(target.channel_name)に「\(content)」を投稿しますか？")
        Self.logger.notice("チャンネル投稿の確認が完了しました。")
        try await actions.send(content, to: channel.channel)
        Self.logger.notice("チャンネル投稿が完了しました。")
        return .result(dialog: "#\(target.channel_name)に投稿しました。")
    }
}

struct StartZooCallIntent: AudioPlaybackIntent {
    private static let logger = Logger(subsystem: "works.zli.zoo", category: "Siri")
    static let title: LocalizedStringResource = "Zooで電話をかける"
    static let description = IntentDescription("Zooのメンバーを表示名またはユーザー名で指定し、その人とのDMで音声通話を始めます。電話番号は使いません。ビデオ通話には対応していません。")
    // 発信確認をSiri内で済ませてから、CallKitの通話画面を開く。
    static var supportedModes: IntentModes { .foreground(.dynamic) }
    static var authenticationPolicy: IntentAuthenticationPolicy { .requiresAuthentication }

    @Parameter(title: "相手", description: "電話をかけるZooのメンバー。表示名またはユーザー名で1人を指定します。",
               requestValueDialog: "誰に電話をかけますか？") var recipient: ZooRecipientEntity
    static var parameterSummary: some ParameterSummary { Summary("\(\.$recipient)に電話をかける") }

    @MainActor func perform() async throws -> some IntentResult & ProvidesDialog {
        Self.logger.notice("Zooの通話Intentを開始しました。")
        do {
            let workspace = try await AppSession.shared.prepareForShortcut()
            let actions = ShortcutActions(workspace: workspace)
            let person = try await actions.resolve(recipient.recipient)
            Self.logger.notice("通話相手を解決し、発信確認を要求します。")
            try await requestConfirmation(dialog: "Zooで\(person.name)（@\(person.user_name)）さんに音声通話を発信しますか？")
            Self.logger.notice("発信確認が完了しました。Zooを前面に移します。")
            if systemContext.currentMode != .foreground {
                guard systemContext.currentMode.canContinueInForeground else {
                    throw ZooError.auth("通話を始めるには、iPhoneのZooを開いてください。")
                }
                try await continueInForeground(alwaysConfirm: false)
            }
            try await actions.call(recipient.recipient)
            Self.logger.notice("Zooの通話開始が完了しました。")
            return .result(dialog: "\(person.name)さんとの通話を開始しました。")
        } catch {
            Self.logger.error("Zooの通話処理が終了しました。エラー種別: \(String(describing: type(of: error)), privacy: .public)")
            throw error
        }
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
