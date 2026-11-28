import SwiftUI

struct MessagePressFeedback: ViewModifier {
    let isEnabled: Bool
    let isPressing: Bool
    @Environment(\.colorScheme) private var colorScheme

    func body(content: Content) -> some View {
        content
            .overlay {
                RoundedRectangle(cornerRadius: 12)
                    .fill(.black.opacity(colorScheme == .dark ? 0.32 : 0.12))
                    .opacity(isEnabled && isPressing ? 1 : 0)
                    .allowsHitTesting(false)
                    .accessibilityHidden(true)
            }
    }
}
