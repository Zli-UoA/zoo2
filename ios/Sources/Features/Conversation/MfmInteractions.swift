import Foundation
import SwiftUI
import Synchronization

struct MfmLinkAttribute: TextAttribute { let url: URL }
struct MfmEmbeddedLinksAttribute: TextAttribute {
    let size: CGSize
    let regions: [MfmInteractionMap.Region]
    static func == (lhs: Self, rhs: Self) -> Bool { lhs.size == rhs.size && lhs.regions == rhs.regions }
    func hash(into hasher: inout Hasher) {
        hasher.combine(size.width); hasher.combine(size.height)
        for region in regions {
            hasher.combine(region.bounds.minX); hasher.combine(region.bounds.minY)
            hasher.combine(region.bounds.width); hasher.combine(region.bounds.height); hasher.combine(region.url)
        }
    }
}
struct MfmImageLink: Hashable, Identifiable {
    let url: URL
    let label: String
    var id: String { url.absoluteString + "|" + label }
}

final class MfmInteractionMap: Sendable {
    struct Region: Sendable, Equatable { let bounds: CGRect; let url: URL }
    private struct State: Sendable { var regions: [Region] = []; var offset = CGPoint.zero }
    private let state = Mutex(State())
    func setOffset(_ offset: CGPoint) { state.withLock { $0.offset = offset } }
    func update(_ values: [Region]) { state.withLock { $0.regions = values } }
    func snapshot() -> [Region] {
        state.withLock { state in state.regions.map { .init(bounds: $0.bounds.offsetBy(dx: state.offset.x, dy: state.offset.y), url: $0.url) } }
    }
    func url(at point: CGPoint) -> URL? { snapshot().last { $0.bounds.contains(point) }?.url }
}

struct MfmRubyLayout: Layout {
    let interactions: MfmInteractionMap
    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let sizes = subviews.map { $0.sizeThatFits(proposal) }
        return CGSize(width: sizes.map(\.width).max() ?? 0, height: sizes.reduce(0) { $0 + $1.height })
    }
    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for (index, view) in subviews.enumerated() {
            let size = view.sizeThatFits(proposal)
            let x = bounds.midX - size.width / 2
            if index == 1 { interactions.setOffset(CGPoint(x: x - bounds.minX, y: y - bounds.minY)) }
            view.place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
            y += size.height
        }
    }
}
