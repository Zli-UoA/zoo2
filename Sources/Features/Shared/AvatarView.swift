import SwiftUI

struct AvatarView: View {
    static let size: CGFloat = 38
    let name: String
    var url: URL? = nil
    var dimension: CGFloat = Self.size

    var body: some View {
        AsyncImage(url: url) { phase in
            if let image = phase.image {
                image.resizable().scaledToFill()
            } else {
                Text(String(name.prefix(1)))
                    .font(.system(size: dimension * 0.4, weight: .semibold))
                    .lineLimit(1).minimumScaleFactor(0.25).foregroundStyle(.secondary)
            }
        }
        .id(url)
        .frame(width: dimension, height: dimension)
        .background(AppDesign.surface)
        .clipShape(Circle())
        .accessibilityHidden(true)
    }
}
