import Foundation

@concurrent func encodeSyncValue<T: Encodable & Sendable>(_ value: T) async throws -> Data {
    try JSONEncoder().encode(value)
}

extension Optional where Wrapped == Data {
    @concurrent func asyncDecode<T: Decodable & Sendable>(_ type: T.Type) async throws -> T? {
        try map { try JSONDecoder().decode(type, from: $0) }
    }
}

func isOfflineFailure(_ error: Error) -> Bool {
    guard let error = error as? URLError else { return false }
    return [.notConnectedToInternet, .networkConnectionLost, .timedOut, .cannotFindHost, .cannotConnectToHost,
            .dnsLookupFailed, .internationalRoamingOff, .dataNotAllowed].contains(error.code)
}
