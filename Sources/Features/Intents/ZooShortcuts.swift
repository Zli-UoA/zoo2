import AppIntents

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
        AppShortcut(intent: StartZooCallIntent(), phrases: [
            "\(.applicationName)で\(\.$recipient)に電話をかけて",
            "\(.applicationName)で電話をかけて"
        ], shortTitle: "電話をかける", systemImageName: "phone")
    }
}
