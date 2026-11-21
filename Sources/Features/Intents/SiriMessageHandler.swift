import Intents

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

// SiriKitのコールバックをまたぐ値は、不変のスナップショットにしてMainActorへ渡す。
private struct SiriMessageRequest: Sendable {
    struct Recipient: Sendable {
        let identifier: String?
        let name: String
        let handle: String?
        let matches: [SiriRecipientMatch]
    }
    let recipients: [Recipient]
    let content: String?
    let supported: Bool

    init(_ intent: INSendMessageIntent) {
        recipients = (intent.recipients ?? []).map {
            Recipient(identifier: $0.customIdentifier, name: $0.displayName, handle: $0.personHandle?.value,
                      matches: ($0.siriMatches ?? []).map(SiriRecipientMatch.init))
        }
        content = intent.content
        supported = (intent.outgoingMessageType == .outgoingMessageText || intent.outgoingMessageType == .unknown) &&
            (intent.attachments ?? []).isEmpty && intent.speakableGroupName == nil && intent.conversationIdentifier == nil
    }
}

// INIntent自体は呼び出し元の隔離領域に残し、必要な値だけを送受信する。
final class SiriMessageHandler: NSObject, Sendable, INSendMessageIntentHandling {
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
                recipient.exactlyMatches(query)
            }
        }
        return exact.isEmpty ? recipients.filter { recipient in queries.contains { recipient.matches($0) } } : exact
    }

    @MainActor private func process(_ request: SiriMessageRequest, send: Bool) async -> INSendMessageIntentResponseCode {
        guard request.supported, request.recipients.count == 1, let input = request.recipients.first,
              let identifier = input.identifier, let content = request.content else { return .failure }
        do {
            try ShortcutActions.validateMessage(content)
            let workspace = try await prepare()
            let actions = ShortcutActions(workspace: workspace)
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
