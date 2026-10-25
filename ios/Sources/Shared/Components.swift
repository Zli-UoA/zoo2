import SwiftUI

extension Color {
    // sRGB equivalents of the semantic tokens in frontend/src/styles.css.
    static let zooAccent = adaptive(light: (0, 0.5011, 0.2829), dark: (0.2719, 0.7472, 0.5109))
    static let zooBrandForeground = adaptive(light: (0.9603, 0.9893, 0.9706), dark: (0.01, 0.0669, 0.0325))
    static let zooBrandMuted = adaptive(light: (0.8521, 0.9694, 0.8957), dark: (0.082, 0.2092, 0.1413))
    static let zooBackground = adaptive(light: (1, 1, 1), dark: (0.039, 0.039, 0.039))
    static let zooSidebar = adaptive(light: (0.98, 0.98, 0.98), dark: (0.09, 0.09, 0.09))
    static let zooMuted = adaptive(light: (0.961, 0.961, 0.961), dark: (0.149, 0.149, 0.149))
    static let zooBorder = adaptive(light: (0.898, 0.898, 0.898), dark: (0.22, 0.22, 0.22))

    private static func adaptive(light: (Double, Double, Double), dark: (Double, Double, Double)) -> Color {
        Color(uiColor: UIColor { traits in
            let value = traits.userInterfaceStyle == .dark ? dark : light
            return UIColor(red: value.0, green: value.1, blue: value.2, alpha: 1)
        })
    }
}

struct ZooMark: View {
    var size: CGFloat = 64
    var body: some View {
        Image(systemName: "pawprint.fill").font(.system(size: size * 0.55, weight: .medium))
            .foregroundStyle(Color.zooBrandForeground).frame(width: size, height: size)
            .background(Color.zooAccent, in: RoundedRectangle(cornerRadius: size * 0.28))
            .accessibilityLabel("Zoo")
    }
}

struct Avatar: View {
    let name: String
    var size: CGFloat = 40
    var body: some View {
        Text(String(name.prefix(1)).uppercased()).font(.system(size: size * 0.42, weight: .semibold, design: .rounded))
            .foregroundStyle(.secondary)
            .frame(width: size, height: size)
            .background(Color.zooMuted, in: Circle())
            .accessibilityHidden(true)
    }
}

struct LabeledTextField: View {
    let title: String
    @Binding var text: String
    var prompt: String = ""
    var axis: Axis = .horizontal
    var identifier: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(title).font(.subheadline.weight(.medium))
            TextField(prompt, text: $text, axis: axis)
                .lineLimit(1...4)
                .accessibilityLabel(title)
                .accessibilityIdentifier(identifier ?? title)
        }.padding(.vertical, 4)
    }
}

struct ConversationDateDivider: View {
    let date: Date
    var body: some View {
        HStack(spacing: 12) {
            Rectangle().fill(Color.zooBorder).frame(height: 1)
            Text(date.formatted(.dateTime.year().month().day().weekday(.abbreviated).locale(Locale(identifier: "ja_JP"))))
                .font(.caption.weight(.medium)).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .layoutPriority(1)
            Rectangle().fill(Color.zooBorder).frame(height: 1)
        }.padding(.horizontal, 16).padding(.vertical, 12)
        .accessibilityElement(children: .combine)
    }
}
