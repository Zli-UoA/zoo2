import Foundation

struct Envelope<T: Decodable & Sendable>: Decodable, Sendable {
    let data: T
    let txid: String?
}
