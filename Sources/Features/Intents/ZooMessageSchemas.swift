import AppIntents
import CoreSpotlight
import CoreTransferable
import GeoToolbox
import LinkPresentation
import os

@available(iOS 27.0, *)
@AppEntity(schema: .messages.messagePerson)
struct ZooMessagePerson: IndexedEntity, Transferable {
    static let defaultQuery = ZooMessagePersonQuery()
    let id: String
    var person: IntentPerson

    static var transferRepresentation: some TransferRepresentation {
        IntentValueRepresentation(exporting: \.person)
    }

    init(_ recipient: ShortcutRecipient) {
        id = recipient.id
        person = IntentPerson(identifier: .applicationDefined(recipient.id), name: .displayName(recipient.name),
                              handle: .init(applicationDefined: recipient.username))
    }

    @MainActor static func currentUser(in workspace: WorkspaceStore) -> Self {
        let user = workspace.user
        return Self(.init(id: "\(workspace.client.serverURL)|\(user.id)|\(user.id)",
                          personID: user.id, name: user.name, username: user.user_name))
    }

    static func matching(_ person: IntentPerson, in recipients: [ShortcutRecipient]) -> [ShortcutRecipient] {
        guard !person.isMe else { return [] }
        if case .applicationDefined(let id) = person.identifier {
            let matches = recipients.filter { $0.id == id }
            // ZooのIDはアカウント付き。Siri由来の別形式のIDだけは名前・ハンドルで照合する。
            if !matches.isEmpty || id.contains("|") { return matches }
        }
        if case .applicationDefined(let username) = person.handle?.value {
            let matches = recipients.filter { $0.username.localizedCaseInsensitiveCompare(username) == .orderedSame }
            if !matches.isEmpty { return matches }
        }
        let name: String
        switch person.name {
        case .displayName(let value): name = value.trimmingCharacters(in: .whitespacesAndNewlines)
        case .components(let value): name = PersonNameComponentsFormatter.localizedString(from: value, style: .default)
        case .unknown: name = ""
        @unknown default: name = ""
        }
        return name.isEmpty ? [] : recipients.filter {
            $0.name.localizedCaseInsensitiveCompare(name) == .orderedSame || $0.username.localizedCaseInsensitiveCompare(name) == .orderedSame
        }
    }

    var displayRepresentation: DisplayRepresentation {
        let name: String
        switch person.name {
        case .displayName(let value): name = value
        case .components(let components): name = PersonNameComponentsFormatter.localizedString(from: components, style: .default)
        case .unknown: name = "Zooのメンバー"
        @unknown default: name = "Zooのメンバー"
        }
        let username: String
        if case .applicationDefined(let value) = person.handle?.value { username = value } else { username = "" }
        return DisplayRepresentation(title: "\(name)", subtitle: "@\(username)")
    }
}

@available(iOS 27.0, *)
struct ZooMessagePersonQuery: EntityStringQuery, IntentValueQuery {
    @MainActor func values(for input: [IntentPerson]) async throws -> [ZooMessagePerson] {
        Logger(subsystem: "works.zli.zoo", category: "Siri").notice("Siriの連絡先をZooの宛先に変換します。")
        let workspace = try await AppSession.shared.prepareForShortcut()
        let recipients = try await ShortcutActions(workspace: workspace).recipients()
        let result = values(for: input, in: recipients)
        #if DEBUG
        let logger = Logger(subsystem: "works.zli.zoo", category: "Siri")
        logger.notice("Siri宛先変換の件数: 入力=\(input.count)、候補=\(recipients.count)、結果=\(result.count)")
        #endif
        return result
    }

    func values(for input: [IntentPerson], in recipients: [ShortcutRecipient]) -> [ZooMessagePerson] {
        let matches = input.map { ZooMessagePerson.matching($0, in: recipients) }
        // 一部だけを解決して、複数宛先の要求を1人への送信に変えない。
        guard matches.allSatisfy({ !$0.isEmpty }) else { return [] }
        var ids: Set<String> = []
        return matches.flatMap { $0 }
            .filter { ids.insert($0.id).inserted }.map(ZooMessagePerson.init)
    }

    @MainActor func entities(for identifiers: [String]) async throws -> [ZooMessagePerson] {
        let workspace = try await AppSession.shared.prepareForShortcut()
        return try await entities(for: identifiers, in: workspace)
    }

    @MainActor func entities(for identifiers: [String], in workspace: WorkspaceStore) async throws -> [ZooMessagePerson] {
        let author = ZooMessagePerson.currentUser(in: workspace)
        let ids = Set(identifiers)
        var people = ids.contains(author.id) ? [author] : []
        if ids.contains(where: { $0 != author.id }) {
            people += try await ShortcutActions(workspace: workspace).recipients()
                .filter { ids.contains($0.id) }.map(ZooMessagePerson.init)
        }
        return people
    }

    func entities(matching string: String) async throws -> [ZooMessagePerson] {
        try await ZooRecipientQuery().entities(matching: string).map { ZooMessagePerson($0.recipient) }
    }

    func suggestedEntities() async throws -> [ZooMessagePerson] {
        try await ZooRecipientQuery().suggestedEntities().map { ZooMessagePerson($0.recipient) }
    }
}

@available(iOS 27.0, *)
@UnionValue
enum ZooMessageDestination {
    case member(ZooMessagePerson)
    case members([ZooMessagePerson])

    @MainActor func resolve(in workspace: WorkspaceStore) async throws -> ShortcutRecipient {
        let recipients = try await ShortcutActions(workspace: workspace).recipients()
        return try resolve(in: recipients)
    }

    func resolve(in recipients: [ShortcutRecipient]) throws -> ShortcutRecipient {
        let matches: [ShortcutRecipient]
        switch self {
        case .member(let member): matches = recipients.filter { $0.id == member.id }
        case .members(let values):
            guard values.count == 1, let member = values.first else { throw Self.recipientCountError }
            matches = recipients.filter { $0.id == member.id }
        }
        guard matches.count == 1, let recipient = matches.first else { throw Self.recipientError }
        return recipient
    }

    private static var recipientCountError: ZooError { .auth("Siriからの送信先は1人を指定してください。") }
    private static var recipientError: ZooError { .auth("宛先を一人に特定できません。Zooのユーザー名で指定してください。") }
}

// 送信結果の表示に必要な型。未対応の添付・装飾はスキーマに従って空値で返す。
@available(iOS 27.0, *)
@AppEnum(schema: .messages.messageType)
enum ZooSchemaMessageType: String {
    case unspecified, text
    static let caseDisplayRepresentations: [Self: DisplayRepresentation] = [.unspecified: "未指定", .text: "テキスト"]
}

@available(iOS 27.0, *)
@AppEnum(schema: .messages.messageAttribute)
enum ZooSchemaMessageAttribute: String {
    case sent
    static let caseDisplayRepresentations: [Self: DisplayRepresentation] = [.sent: "送信済み"]
}

@available(iOS 27.0, *)
@AppEnum(schema: .messages.conversationAttribute)
enum ZooSchemaConversationAttribute: String {
    case directMessage
    static let caseDisplayRepresentations: [Self: DisplayRepresentation] = [.directMessage: "DM"]
}

@available(iOS 27.0, *)
@AppEnum(schema: .messages.messageEffect)
enum ZooSchemaMessageEffect: String {
    case none
    static let caseDisplayRepresentations: [Self: DisplayRepresentation] = [.none: "なし"]
}

@available(iOS 27.0, *)
@AppEntity(schema: .messages.customAttachment)
struct ZooSchemaCustomAttachment: AppEntity {
    static let defaultQuery = CustomAttachmentQuery()
    let id: String
    var sourceName: AttributedString? { nil }
    var description: AttributedString? { nil }
    var displayRepresentation: DisplayRepresentation { "添付ファイル" }
    struct CustomAttachmentQuery: EntityQuery {
        func entities(for identifiers: [String]) async throws -> [ZooSchemaCustomAttachment] { [] }
    }
}

@available(iOS 27.0, *)
@UnionValue
enum ZooSchemaReaction { case text(AttributedString) }

@available(iOS 27.0, *)
@AppEntity(schema: .messages.conversation)
struct ZooSentConversation: AppEntity {
    static let defaultQuery = SentConversationQuery()
    let id: String
    var recipients: [ZooMessagePerson]
    var displayName: String
    var previewText: AttributedString
    var conversationName: String? { nil }
    var isRead: Bool { true }
    var attributes: Set<ZooSchemaConversationAttribute> { [.directMessage] }
    var dateLastActive: Date?
    var displayRepresentation: DisplayRepresentation { DisplayRepresentation(title: "\(displayName)") }

    init(id: String, recipients: [ZooMessagePerson], displayName: String, previewText: AttributedString, dateLastActive: Date?) {
        self.id = id
        self.recipients = recipients
        self.displayName = displayName
        self.previewText = previewText
        self.dateLastActive = dateLastActive
    }
    struct SentConversationQuery: EntityQuery {
        @MainActor func entities(for identifiers: [String]) async throws -> [ZooSentConversation] {
            let workspace = try await AppSession.shared.prepareForShortcut()
            return ZooSentMessageStore.shared.conversations(for: identifiers, in: workspace)
        }
    }
}

@available(iOS 27.0, *)
@AppEntity(schema: .messages.message)
struct ZooSentMessage: AppEntity {
    static let defaultQuery = SentMessageQuery()
    let id: String
    var body: AttributedString?
    var date: Date
    var author: ZooMessagePerson
    var conversation: ZooSentConversation
    var messageType: ZooSchemaMessageType { .text }
    var isRead: Bool { false }
    var attributes: Set<ZooSchemaMessageAttribute> { [.sent] }
    var subject: AttributedString? { nil }
    var attachments: [IntentFile] { [] }
    var audioMessage: IntentFile? { nil }
    var customAttachments: [ZooSchemaCustomAttachment] { [] }
    var locations: [PlaceDescriptor] { [] }
    var links: [LinkPresentation.LinkMetadata] { [] }
    var messageEffect: ZooSchemaMessageEffect? { nil }
    var reaction: ZooSchemaReaction? { nil }
    var referencedMessage: ZooSentMessage? { nil }
    var notificationIdentifier: String? { nil }
    var displayRepresentation: DisplayRepresentation { DisplayRepresentation(title: "\(body ?? "")") }

    @MainActor init(_ message: Message, recipient: ShortcutRecipient, workspace: WorkspaceStore) {
        let account = "\(workspace.client.serverURL)|\(workspace.user.id)"
        id = "\(account)|\(message.id)"
        body = AttributedString(message.content)
        date = message.date ?? .now
        author = ZooMessagePerson.currentUser(in: workspace)
        conversation = ZooSentConversation(id: "\(account)|\(message.channel_id)", recipients: [ZooMessagePerson(recipient)],
                                           displayName: recipient.name, previewText: AttributedString(message.content), dateLastActive: date)
    }

    struct SentMessageQuery: EntityQuery {
        @MainActor func entities(for identifiers: [String]) async throws -> [ZooSentMessage] {
            let workspace = try await AppSession.shared.prepareForShortcut()
            return ZooSentMessageStore.shared.messages(for: identifiers, in: workspace)
        }
    }
}

// Siriが直近の送信結果を再取得できるよう、同じログイン中だけ少数の結果を保持する。
@available(iOS 27.0, *)
@MainActor
final class ZooSentMessageStore {
    static let shared = ZooSentMessageStore()
    private weak var workspace: WorkspaceStore?
    private var results: [ZooSentMessage] = []

    func remember(_ message: ZooSentMessage, in workspace: WorkspaceStore) {
        guard workspace.active else { return }
        if self.workspace !== workspace { results.removeAll(); self.workspace = workspace }
        results.removeAll { $0.id == message.id }
        results.append(message)
        if results.count > 20 { results.removeFirst(results.count - 20) }
    }

    func messages(for identifiers: [String], in workspace: WorkspaceStore) -> [ZooSentMessage] {
        guard self.workspace === workspace, workspace.active else { return [] }
        return identifiers.compactMap { id in results.first { $0.id == id } }
    }

    func conversations(for identifiers: [String], in workspace: WorkspaceStore) -> [ZooSentConversation] {
        guard self.workspace === workspace, workspace.active else { return [] }
        return identifiers.compactMap { id in results.last { $0.conversation.id == id }?.conversation }
    }

    func clear(in workspace: WorkspaceStore) {
        guard self.workspace === workspace else { return }
        results.removeAll()
        self.workspace = nil
    }
}

@available(iOS 27.0, *)
@AppIntent(schema: .messages.sendMessage)
struct SendZooSchemaMessageIntent: AppIntent {
    static let title: LocalizedStringResource = "ZooでDMを送信する"
    static let isAssistantOnly = true
    private static let logger = Logger(subsystem: "works.zli.zoo", category: "Siri")
    static var authenticationPolicy: IntentAuthenticationPolicy { .requiresAuthentication }
    var destination: ZooMessageDestination
    var content: AttributedString?
    var subject: AttributedString?
    var attachments: [IntentFile]
    var audioMessage: IntentFile?
    var locations: [PlaceDescriptor]
    var links: [URL]
    var scheduledDate: Date?

    init() { attachments = []; locations = []; links = [] }

    @MainActor func perform() async throws -> some ReturnsValue<[ZooSentMessage]> {
        Self.logger.notice("Siri AIからの送信処理を開始しました。")
        let text = try validatedContent()
        try ShortcutActions.validateMessage(text)
        let workspace = try await AppSession.shared.prepareForShortcut()
        let recipient = try await destination.resolve(in: workspace)
        let message = try await ShortcutActions(workspace: workspace).send(text, to: recipient)
        let result = ZooSentMessage(message, recipient: recipient, workspace: workspace)
        ZooSentMessageStore.shared.remember(result, in: workspace)
        Self.logger.notice("Siri AIからの送信が完了しました。")
        return .result(value: [result])
    }

    func validatedContent() throws -> String {
        try ZooSchemaContent.text(content, subject: subject, attachments: attachments, audioMessage: audioMessage,
                                  locations: locations, links: links, scheduledDate: scheduledDate)
    }
}

@available(iOS 27.0, *)
@AppIntent(schema: .messages.draftMessage)
struct DraftZooSchemaMessageIntent: AppIntent {
    static let title: LocalizedStringResource = "ZooでDMの下書きを作成する"
    static let isAssistantOnly = true
    static var supportedModes: IntentModes { .foreground }
    static var authenticationPolicy: IntentAuthenticationPolicy { .requiresAuthentication }
    var destination: ZooMessageDestination?
    var content: AttributedString?
    var subject: AttributedString?
    var attachments: [IntentFile]
    var audioMessage: IntentFile?
    var locations: [PlaceDescriptor]
    var links: [URL]
    var scheduledDate: Date?

    init() { attachments = []; locations = []; links = [] }

    @MainActor func perform() async throws -> some IntentResult {
        Logger(subsystem: "works.zli.zoo", category: "Siri").notice("Siri AIからの下書き処理を開始しました。")
        let text = try ZooSchemaContent.text(content, subject: subject, attachments: attachments, audioMessage: audioMessage,
                                             locations: locations, links: links, scheduledDate: scheduledDate)
        let workspace = try await AppSession.shared.prepareForShortcut()
        let recipient = try await destination?.resolve(in: workspace)
        try await ShortcutActions(workspace: workspace).draft(text, to: recipient)
        Logger(subsystem: "works.zli.zoo", category: "Siri").notice("Siri AIからの下書き作成が完了しました。")
        return .result()
    }
}

@available(iOS 27.0, *)
enum ZooSchemaContent {
    static func text(_ content: AttributedString?, subject: AttributedString?, attachments: [IntentFile], audioMessage: IntentFile?,
                     locations: [PlaceDescriptor], links: [URL], scheduledDate: Date?) throws -> String {
        guard subject == nil, attachments.isEmpty, audioMessage == nil, locations.isEmpty, links.isEmpty, scheduledDate == nil else {
            throw ZooError.auth("SiriからはテキストのDMを送れます。添付・位置情報・予約送信はZooで確認してください。")
        }
        return content.map { String($0.characters) } ?? ""
    }
}
