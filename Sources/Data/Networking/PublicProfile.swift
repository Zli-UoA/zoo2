import Foundation

struct PublicProfile: Codable, Equatable, Sendable {
    var display_name: String
    let default_display_name: String
    var summary: String

    var name: String { display_name.isEmpty ? default_display_name : display_name }

    var canSave: Bool {
        let name = display_name.trimmingCharacters(in: .whitespacesAndNewlines)
        return name.unicodeScalars.count <= 100 && !name.unicodeScalars.contains { $0.properties.generalCategory == .control }
            && summary.unicodeScalars.count <= 1000 && !summary.contains("\0")
    }
}

extension NativeChatClient {
    var accountProfileURL: URL? {
        guard let issuer = try? ConnectionSettings.secureURL(api.settings.issuer) else { return nil }
        return URL(string: "/settings/profile", relativeTo: issuer)?.absoluteURL
    }

    func publicProfile() async throws -> PublicProfile {
        try await api.get("api/activitypub/profile")
    }

    func updatePublicProfile(name: String, summary: String) async throws -> PublicProfile {
        let data = try await api.data("api/activitypub/profile", method: "PUT",
                                      body: ["display_name": .string(name), "summary": .string(summary)])
        return try JSONDecoder().decode(PublicProfile.self, from: data)
    }
}

#if DEBUG
extension PreviewChatClient {
    var accountProfileURL: URL? { URL(string: "https://account.test.invalid/settings/profile") }

    func publicProfile() async throws -> PublicProfile { previewProfile }

    func updatePublicProfile(name: String, summary: String) async throws -> PublicProfile {
        previewProfile.display_name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        previewProfile.summary = summary
        return previewProfile
    }
}
#endif
