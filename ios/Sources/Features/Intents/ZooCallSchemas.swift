import AppIntents
import CoreSpotlight
import CoreTransferable
import os

@available(iOS 27.0, *)
@AppEntity(schema: .phone.phonePerson)
struct ZooPhonePerson: IndexedEntity, Transferable {
    static let defaultQuery = ZooPhonePersonQuery()
    let id: String
    var person: IntentPerson

    init(_ member: ZooMessagePerson) { id = member.id; person = member.person }
    init(_ recipient: ShortcutRecipient) { self.init(ZooMessagePerson(recipient)) }

    static var transferRepresentation: some TransferRepresentation {
        IntentValueRepresentation(exporting: \.person)
    }

    var displayRepresentation: DisplayRepresentation { ZooMessagePerson.displayRepresentation(for: person) }
}

@available(iOS 27.0, *)
struct ZooPhonePersonQuery: EntityStringQuery, EnumerableEntityQuery, IntentValueQuery {
    func values(for input: [IntentPerson]) async throws -> [ZooPhonePerson] {
        try await ZooMessagePersonQuery().values(for: input).map(ZooPhonePerson.init)
    }

    func entities(for identifiers: [String]) async throws -> [ZooPhonePerson] {
        try await ZooMessagePersonQuery().entities(for: identifiers).map(ZooPhonePerson.init)
    }

    func entities(matching string: String) async throws -> [ZooPhonePerson] {
        try await ZooRecipientQuery().entities(matching: string).map { ZooPhonePerson($0.recipient) }
    }

    func allEntities() async throws -> [ZooPhonePerson] {
        try await ZooRecipientQuery().allEntities().map { ZooPhonePerson($0.recipient) }
    }

    func suggestedEntities() async throws -> [ZooPhonePerson] {
        try await ZooRecipientQuery().suggestedEntities().map { ZooPhonePerson($0.recipient) }
    }
}

@available(iOS 27.0, *)
@UnionValue
enum ZooCallDestination {
    case person(ZooPhonePerson)
    case people([ZooPhonePerson])

    func resolve(in recipients: [ShortcutRecipient]) throws -> ShortcutRecipient {
        let people: [ZooPhonePerson]
        switch self {
        case .person(let person): people = [person]
        case .people(let values): people = values
        }
        guard people.count == 1, let person = people.first else {
            throw ZooError.auth("Siriからの通話相手は1人を指定してください。")
        }
        let matches = recipients.filter { $0.id == person.id }
        guard matches.count == 1, let recipient = matches.first else {
            throw ZooError.auth("通話相手を一人に特定できません。Zooのユーザー名で指定してください。")
        }
        return recipient
    }
}

@available(iOS 27.0, *)
@AppEnum(schema: .phone.audioVisualMode)
enum ZooCallAudioVisualMode: String {
    case audio, video
    static let caseDisplayRepresentations: [Self: DisplayRepresentation] = [.audio: "音声", .video: "ビデオ"]

    func validate() throws {
        guard self == .audio else { throw ZooError.auth("Zooは音声通話のみ対応しています。") }
    }
}

@available(iOS 27.0, *)
@AppIntent(schema: .phone.startCall)
struct StartZooSchemaCallIntent: AudioRecordingIntent, AudioPlaybackIntent {
    static let title: LocalizedStringResource = "Zooで音声通話を始める"
    static let description = IntentDescription("Zooのメンバーを表示名またはユーザー名で指定し、その人とのDMで音声通話を始めます。")
    static var supportedModes: IntentModes { .foreground(.dynamic) }
    static var authenticationPolicy: IntentAuthenticationPolicy { .requiresAuthentication }
    private static let logger = Logger(subsystem: "works.zli.zoo", category: "Siri")

    var destination: ZooCallDestination
    @Parameter(title: "通話の種類", default: .audio)
    var audioVisualMode: ZooCallAudioVisualMode

    @MainActor func perform() async throws -> some IntentResult & ProvidesDialog {
        Self.logger.notice("Siri AIからの通話処理を開始しました。")
        do {
            try audioVisualMode.validate()
            let workspace = try await AppSession.shared.prepareForShortcut()
            let actions = ShortcutActions(workspace: workspace)
            let recipient = try await destination.resolve(in: actions.recipients())
            let person = try await actions.resolve(recipient)
            Self.logger.notice("通話相手を解決し、発信確認を要求します。")
            try await requestConfirmation(dialog: "Zooで\(person.name)（@\(person.user_name)）さんに音声通話を発信しますか？")
            if systemContext.currentMode != .foreground {
                guard systemContext.currentMode.canContinueInForeground else {
                    throw ZooError.auth("通話を始めるには、iPhoneのZooを開いてください。")
                }
                try await continueInForeground(alwaysConfirm: false)
            }
            Self.logger.notice("発信確認が完了しました。通話中の表示と音声を開始します。")
            try await actions.call(recipient, requiresLiveActivity: true)
            Self.logger.notice("Siri AIからの通話開始が完了しました。")
            return .result(dialog: "\(person.name)さんとの通話を開始しました。")
        } catch {
            Self.logger.error("Siri AIからの通話処理が終了しました。エラー種別: \(String(describing: type(of: error)), privacy: .public)")
            throw error
        }
    }
}
