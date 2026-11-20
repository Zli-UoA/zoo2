import Intents
import os

struct SiriRecipientMatch: Sendable {
    let identifier: String?
    let name: String
    let handle: String?

    init(_ person: INPerson) {
        identifier = person.customIdentifier
        name = person.displayName
        handle = person.personHandle?.value
    }
}

struct SiriChannelMatch: Sendable {
    let identifier: String?
    let name: String

    init(_ value: any INSpeakable) {
        // 名前だけのINSpeakableStringも、同じ文字列を識別子として返すことがある。
        // Zooのアカウント付きIDだけを確定済みの照合に使う。
        identifier = value.vocabularyIdentifier.flatMap { $0 != value.spokenPhrase && $0.contains("|") ? $0 : nil }
        name = value.spokenPhrase
    }
}

enum SiriChannelResolution: Equatable, Sendable {
    case notRequired
    case unsupported
    case success(ShortcutChannel)
    case disambiguation([ShortcutChannel])
    case confirmationRequired(ShortcutChannel)
}

// SiriKitのコールバックをまたぐ値は、不変のスナップショットにしてMainActorへ渡す。
struct SiriMessageRequest: Sendable {
    struct Recipient: Sendable {
        let identifier: String?
        let name: String
        let handle: String?
        let matches: [SiriRecipientMatch]
    }
    let recipients: [Recipient]
    let content: String?
    let supported: Bool
    let groupName: String?
    let groupIdentifier: String?
    let groupMatches: [SiriChannelMatch]
    let conversationIdentifier: String?
    var isChannel: Bool { groupName != nil || conversationIdentifier != nil }

    init(_ intent: INSendMessageIntent) {
        let recipients = (intent.recipients ?? []).map {
            Recipient(identifier: $0.customIdentifier, name: $0.displayName, handle: $0.personHandle?.value,
                      matches: ($0.siriMatches ?? []).map(SiriRecipientMatch.init))
        }
        let supported = (intent.outgoingMessageType == .outgoingMessageText || intent.outgoingMessageType == .unknown) &&
            (intent.attachments ?? []).isEmpty
        self.init(recipients: recipients, content: intent.content, supported: supported,
                  group: intent.speakableGroupName, conversationIdentifier: intent.conversationIdentifier)
    }

    init(recipients: [Recipient], content: String?, supported: Bool, group: (any INSpeakable)?, conversationIdentifier: String?) {
        self.recipients = recipients
        self.content = content
        self.supported = supported
        let match = group.map(SiriChannelMatch.init)
        groupName = match?.name
        groupIdentifier = match?.identifier
        groupMatches = (group?.alternativeSpeakableMatches ?? []).map(SiriChannelMatch.init)
        self.conversationIdentifier = conversationIdentifier
    }
}

// INIntent自体は呼び出し元の隔離領域に残し、必要な値だけを送受信する。
final class SiriMessageHandler: NSObject, Sendable, INSendMessageIntentHandling {
    private static let logger = Logger(subsystem: "works.zli.zoo", category: "Siri")
    private let prepare: @MainActor @Sendable () async throws -> WorkspaceStore

    init(prepare: @escaping @MainActor @Sendable () async throws -> WorkspaceStore = {
        try await AppSession.shared.prepareForShortcut()
    }) {
        self.prepare = prepare
        super.init()
    }

    nonisolated(nonsending) func resolveRecipients(for intent: INSendMessageIntent) async -> [INSendMessageRecipientResolutionResult] {
        let request = SiriMessageRequest(intent)
        guard request.supported, request.recipients.count <= 1 else { return [.unsupported()] }
        if request.isChannel { return request.recipients.isEmpty ? [.notRequired()] : [.unsupported()] }
        guard let input = request.recipients.first else { return [.needsValue()] }
        do {
            let candidates = try await candidates(for: input)
            let people = candidates.map { recipient in
                INPerson(personHandle: INPersonHandle(value: recipient.username, type: .unknown), nameComponents: nil,
                         displayName: candidates.count > 1 ? "\(recipient.name)（@\(recipient.username)）" : recipient.name,
                         image: nil, contactIdentifier: nil, customIdentifier: recipient.id)
            }
            if people.count > 1 { return [.disambiguation(with: people)] }
            if let person = people.first { return [.success(with: person)] }
            return [.unsupported(forReason: .noValidHandle)]
        } catch {
            return [.unsupported(forReason: isOfflineFailure(error) ? .offline : .requiringInAppAuthentication)]
        }
    }

    nonisolated(nonsending) func resolveSpeakableGroupName(for intent: INSendMessageIntent) async -> INSpeakableStringResolutionResult {
        func name(_ channel: ShortcutChannel) -> INSpeakableString {
            INSpeakableString(vocabularyIdentifier: channel.id, spokenPhrase: channel.name, pronunciationHint: nil)
        }
        switch await channelResolution(for: intent) {
        case .notRequired: return .notRequired()
        case .unsupported: return .unsupported()
        case .success(let channel): return .success(with: name(channel))
        case .disambiguation(let channels): return .disambiguation(with: channels.map(name))
        case .confirmationRequired(let channel): return .confirmationRequired(with: name(channel))
        }
    }

    nonisolated(nonsending) func channelResolution(for intent: INSendMessageIntent) async -> SiriChannelResolution {
        await channelResolution(for: SiriMessageRequest(intent))
    }

    nonisolated(nonsending) func channelResolution(for request: SiriMessageRequest) async -> SiriChannelResolution {
        Self.logger.notice("SiriKitのチャンネル検索: グループ指定=\(request.groupName != nil)、会話ID指定=\(request.conversationIdentifier != nil)、代替候補=\(request.groupMatches.count)")
        guard request.supported else { return .unsupported }
        guard request.isChannel else { return .notRequired }
        guard request.recipients.isEmpty else { return .unsupported }
        do {
            let channels = try await channelCandidates(for: request)
            Self.logger.notice("SiriKitのチャンネル検索結果: \(channels.count)件")
            if channels.count > 1 { return .disambiguation(channels) }
            if let channel = channels.first {
                let boundNames: [String]
                if request.groupIdentifier != nil || request.conversationIdentifier != nil {
                    boundNames = request.groupName.map { [$0] } ?? []
                } else {
                    boundNames = request.groupMatches.filter { $0.identifier == channel.id }.map(\.name)
                }
                if boundNames.contains(where: { ShortcutChannel.normalizedName($0) != ShortcutChannel.normalizedName(channel.name) }) {
                    return .confirmationRequired(channel)
                }
                return .success(channel)
            }
            return .unsupported
        } catch {
            Self.logger.error("SiriKitのチャンネル検索に失敗: \(String(describing: type(of: error)), privacy: .public)")
            return .unsupported
        }
    }

    nonisolated(nonsending) func resolveContent(for intent: INSendMessageIntent) async -> INStringResolutionResult {
        guard let content = intent.content, !content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return .needsValue()
        }
        do {
            try ShortcutActions.validateMessage(content)
            return .success(with: content)
        } catch { return .unsupported() }
    }

    nonisolated(nonsending) func resolveOutgoingMessageType(for intent: INSendMessageIntent) async -> INOutgoingMessageTypeResolutionResult {
        if intent.outgoingMessageType == .outgoingMessageText || intent.outgoingMessageType == .unknown {
            return .success(with: .outgoingMessageText)
        }
        return .unsupported()
    }

    nonisolated(nonsending) func confirm(intent: INSendMessageIntent) async -> INSendMessageIntentResponse {
        let code = await process(SiriMessageRequest(intent), send: false)
        return INSendMessageIntentResponse(code: code, userActivity: nil)
    }

    nonisolated(nonsending) func handle(intent: INSendMessageIntent) async -> INSendMessageIntentResponse {
        let code = await process(SiriMessageRequest(intent), send: true)
        return INSendMessageIntentResponse(code: code, userActivity: nil)
    }

    @MainActor private func candidates(for input: SiriMessageRequest.Recipient) async throws -> [ShortcutRecipient] {
        let workspace = try await prepare()
        let recipients = try await ShortcutActions(workspace: workspace).recipients()
        return Self.matchingRecipients(identifier: input.identifier, name: input.name, handle: input.handle,
                                       siriMatches: input.matches, in: recipients)
    }

    @MainActor private func channelCandidates(for request: SiriMessageRequest) async throws -> [ShortcutChannel] {
        if let group = request.groupIdentifier, let conversation = request.conversationIdentifier, group != conversation { return [] }
        let identifier = request.groupIdentifier ?? request.conversationIdentifier
        guard identifier != nil || !request.groupMatches.isEmpty || !ShortcutChannel.normalizedName(request.groupName ?? "").isEmpty else { return [] }
        let workspace = try await prepare()
        let actions = ShortcutActions(workspace: workspace)
        if let identifier { _ = try actions.channelTarget(identifier: identifier, name: "") }
        // 名前の再利用や改名を古いキャッシュで解決すると、別のチャンネルへ送ってしまう。
        let channels = try await actions.channels()
        return Self.matchingChannels(identifier: identifier, name: request.groupName ?? "",
                                     siriMatches: request.conversationIdentifier == nil ? request.groupMatches : [], in: channels)
    }

    static func matchingChannels(identifier: String?, name: String, siriMatches: [SiriChannelMatch] = [],
                                 in channels: [ShortcutChannel]) -> [ShortcutChannel] {
        let primary = ShortcutChannel.matching(identifier: identifier, name: name, in: channels)
        guard !siriMatches.isEmpty else { return primary }
        let alternativeIDs = Set(siriMatches.flatMap {
            ShortcutChannel.matching(identifier: $0.identifier, name: $0.name, in: channels).map(\.id)
        })
        // 失効した識別子しかない場合は、同名の別アカウントの候補へ置き換えない。
        guard identifier != nil || !alternativeIDs.isEmpty else { return [] }
        let ids = alternativeIDs.union(primary.map(\.id))
        return channels.filter { ids.contains($0.id) }
    }

    static func matchingRecipients(identifier: String?, name: String, handle: String?,
                                   siriMatches: [SiriRecipientMatch] = [], in recipients: [ShortcutRecipient]) -> [ShortcutRecipient] {
        if let identifier {
            return recipients.filter { $0.id == identifier }
        }
        if !siriMatches.isEmpty {
            let matchedIDs = Set(siriMatches.flatMap {
                matchingRecipients(identifier: $0.identifier, name: $0.name, handle: $0.handle, in: recipients).map(\.id)
            })
            return recipients.filter { matchedIDs.contains($0.id) }
        }
        let queries = [name, handle ?? ""].map {
            $0.trimmingCharacters(in: .whitespacesAndNewlines)
        }.filter { !$0.isEmpty }
        let exact = recipients.filter { recipient in
            queries.contains { query in
                recipient.name.localizedStandardCompare(query) == .orderedSame ||
                    recipient.username.localizedStandardCompare(query) == .orderedSame
            }
        }
        return exact.isEmpty ? recipients.filter { recipient in queries.contains { recipient.matches($0) } } : exact
    }

    @MainActor func process(_ request: SiriMessageRequest, send: Bool) async -> INSendMessageIntentResponseCode {
        guard request.supported, let content = request.content else { return .failure }
        do {
            try ShortcutActions.validateMessage(content)
            let workspace = try await prepare()
            let actions = ShortcutActions(workspace: workspace)
            if request.isChannel {
                guard request.recipients.isEmpty,
                      let identifier = request.groupIdentifier ?? request.conversationIdentifier,
                      request.conversationIdentifier != nil || request.groupMatches.isEmpty,
                      request.conversationIdentifier == nil || request.conversationIdentifier == identifier else { return .failure }
                let channel = try actions.channelTarget(identifier: identifier, name: request.groupName ?? "")
                if send { try await actions.send(content, to: channel) }
                else { _ = try await actions.resolve(channel) }
                return send ? .success : .ready
            }
            guard request.recipients.count == 1, let identifier = request.recipients.first?.identifier else { return .failure }
            // 解決時のアカウント付き識別子を使い、確認後のアカウント切替でも別人へ送らない。
            guard let recipient = try await actions.recipients().first(where: { $0.id == identifier }) else { return .failure }
            if send { try await actions.send(content, to: recipient) }
            else { _ = try await actions.resolve(recipient) }
            return send ? .success : .ready
        } catch ZooError.expiredSession {
            return .failureRequiringInAppAuthentication
        } catch {
            return .failureRequiringAppLaunch
        }
    }
}
