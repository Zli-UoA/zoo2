import SwiftUI

struct IntegrationCardView: View {
    let card: IntegrationCard
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text([card.source, card.context, card.badge].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · "))
                .font(.caption).foregroundStyle(.secondary)
            if let target = card.destination { Link(card.title, destination: target).font(.subheadline.bold()) }
            else { Text(card.title).font(.subheadline.bold()) }
            if let description = card.description, !description.isEmpty { Text(description).font(.callout).lineLimit(8) }
            if let author = card.author, !author.isEmpty { Text(author).font(.caption).foregroundStyle(.secondary) }
        }.padding(12).frame(maxWidth: .infinity, alignment: .leading)
            .background(Color(uiColor: .secondarySystemBackground), in: RoundedRectangle(cornerRadius: 12))
            .overlay(alignment: .leading) { UnevenRoundedRectangle(topLeadingRadius: 12, bottomLeadingRadius: 12).fill(Color.zooAccent).frame(width: 3) }
    }
}
