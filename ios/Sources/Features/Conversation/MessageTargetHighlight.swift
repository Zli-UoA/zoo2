import SwiftUI

struct MessageTargetHighlight: Equatable {
    static let duration: TimeInterval = 1.35
    let messageID: String
    let generation = UUID()
    private(set) var startedAt: Date?

    mutating func startIfVisible(frame: CGRect?, viewport: CGRect, at date: Date = .now) {
        guard startedAt == nil, let frame, !frame.isEmpty, !viewport.isEmpty, frame.intersects(viewport) else { return }
        startedAt = date
    }

    func intensity(at date: Date, reducedMotion: Bool) -> Double {
        guard let startedAt else { return 0 }
        let elapsed = date.timeIntervalSince(startedAt)
        guard elapsed >= 0, elapsed < Self.duration else { return 0 }
        let rise = reducedMotion ? 1 : min(1, elapsed / 0.18)
        let fall = min(1, (Self.duration - elapsed) / 0.9)
        let value = min(rise, fall)
        return value * value * (3 - 2 * value)
    }
}

struct MessageTargetGlow: View {
    let highlight: MessageTargetHighlight
    @Environment(\.accessibilityReduceMotion) private var reducedMotion

    var body: some View {
        TimelineView(.animation(minimumInterval: 1.0 / 30)) { timeline in
            let intensity = highlight.intensity(at: timeline.date, reducedMotion: reducedMotion)
            RoundedRectangle(cornerRadius: AppDesign.cornerRadius)
                .fill(AppDesign.messageTarget.opacity(0.18 * intensity))
                .overlay(alignment: .leading) {
                    Rectangle().fill(AppDesign.messageTarget.opacity(0.65 * intensity)).frame(width: 3)
                }
                .clipShape(RoundedRectangle(cornerRadius: AppDesign.cornerRadius))
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}
