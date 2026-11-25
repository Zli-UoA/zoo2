import Foundation

struct Message: Codable, Identifiable, Hashable, Sendable {
    let id: String
    var user_id: String
    var channel_id: String
    var content: String
    var created_at: String
    var updated_at: String?
    var deleted_at: String?
    var parent_id: String?
    var card: String?
    var display_data: MessageDisplayData?
    /// 暗号化チャンネルのメッセージ（`content` は空）。Shape は jsonb を JSON か文字列で、bigint を文字列で配る。
    var ciphertext: CiphertextText? = nil
    var epoch: ShapeInteger? = nil
    var revision: ShapeInteger? = nil
    var kind: String? = nil
    /// 復号できなかった理由や、送信者についての注意書き。表示だけに使い、保存も送信もしない。
    var e2eeNotice: String? = nil
    /// `e2eeNotice` のくわしい説明。あれば、注意の横の ⓘ をタップしたときに出す。表示専用。
    var e2eeNoticeDetail: String? = nil
    /// 暗号化に切り替える前の平文のメッセージ（暗号文も `epoch` もない行）。本文はそのまま出し、編集させない。表示専用。
    var e2eeLegacy = false

    var date: Date? { ZooDate.parse(created_at) }

    enum CodingKeys: String, CodingKey {
        case id, user_id, channel_id, content, created_at, updated_at, deleted_at, parent_id, card, display_data
        case ciphertext, epoch, revision, kind
    }
}

/// `ciphertext` の列。文字列ならそのまま、JSON のオブジェクトなら JSON の文字列にする。
struct CiphertextText: Codable, Hashable, Sendable {
    let value: String

    init(_ value: String) { self.value = value }

    init(from decoder: any Decoder) throws {
        let single = try decoder.singleValueContainer()
        if let text = try? single.decode(String.self) { value = text; return }
        let json = try single.decode(JSONValue.self)
        value = String(decoding: try JSONEncoder().encode(json), as: UTF8.self)
    }

    func encode(to encoder: any Encoder) throws {
        var single = encoder.singleValueContainer()
        try single.encode(value)
    }
}

/// bigint の列（`epoch`、`revision`）。Shape は文字列で、REST は数値で返すので、どちらも読む。
struct ShapeInteger: Codable, Hashable, Sendable {
    let value: UInt64

    init(_ value: UInt64) { self.value = value }

    init(from decoder: any Decoder) throws {
        let single = try decoder.singleValueContainer()
        if let number = try? single.decode(UInt64.self) { value = number; return }
        guard let text = try? single.decode(String.self), let number = UInt64(text) else { throw ZooError.invalidResponse }
        value = number
    }

    func encode(to encoder: any Encoder) throws {
        var single = encoder.singleValueContainer()
        try single.encode(value)
    }
}
