import Foundation

enum ZooDate {
    static func parse(_ value: String) -> Date? {
        var normalized = value.replacingOccurrences(of: " ", with: "T")
        if !normalized.hasSuffix("Z"), !normalized.dropFirst(10).contains("+"), !normalized.dropFirst(10).contains("-") {
            normalized += "Z"
        }
        return (try? Date(normalized, strategy: Date.ISO8601FormatStyle(includingFractionalSeconds: true)))
            ?? (try? Date(normalized, strategy: .iso8601))
    }
}
