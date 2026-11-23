import Foundation

struct DeviceAuthorizationPrompt: Equatable, Sendable {
    let userCode: String
    let verificationURI: URL
    let verificationURL: URL
}

struct DeviceAuthorizationResponse: Decodable, Sendable {
    let device_code: String
    let user_code: String
    let verification_uri: String
    let verification_uri_complete: String?
    let expires_in: Int
    let interval: Int?

    func prompt(issuer: String) throws -> DeviceAuthorizationPrompt {
        guard !device_code.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              !user_code.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
              expires_in > 0, (interval ?? 5) > 0 else { throw ZooError.invalidResponse }
        let origin = try ConnectionSettings.secureURL(issuer)
        func verifiedURL(_ raw: String) throws -> URL {
            guard let url = URL(string: raw), url.scheme == "https", url.host?.lowercased() == origin.host?.lowercased(),
                  (url.port ?? 443) == (origin.port ?? 443), url.user == nil, url.password == nil,
                  url.fragment == nil else { throw ZooError.invalidResponse }
            return url
        }
        // 確認コードを第三者のサイトへ渡さず、設定済みの AuthZ 内でだけ承認する。
        let uri = try verifiedURL(verification_uri)
        let complete = try verification_uri_complete.map(verifiedURL) ?? uri
        return DeviceAuthorizationPrompt(userCode: user_code, verificationURI: uri, verificationURL: complete)
    }
}
