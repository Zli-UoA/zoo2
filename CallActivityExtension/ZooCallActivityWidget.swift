import ActivityKit
import SwiftUI
import WidgetKit

@main
struct ZooCallActivityWidget: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: ZooCallActivityAttributes.self) { context in
            HStack(spacing: 12) {
                Image(systemName: "phone.fill").foregroundStyle(.green)
                Text(context.state.ended ? "通話が終了しました" : "Zooの音声通話")
                Spacer()
                if !context.state.ended {
                    Text(context.attributes.startedAt, style: .timer).monospacedDigit()
                }
            }
            .padding()
        } dynamicIsland: { context in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    Label("Zoo", systemImage: "phone.fill").foregroundStyle(.green)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    if !context.state.ended {
                        Text(context.attributes.startedAt, style: .timer).monospacedDigit()
                    }
                }
                DynamicIslandExpandedRegion(.bottom) {
                    Text(context.state.ended ? "通話が終了しました" : "Zooの音声通話")
                }
            } compactLeading: {
                Image(systemName: "phone.fill").foregroundStyle(.green).accessibilityLabel("Zooの音声通話")
            } compactTrailing: {
                Text("Zoo")
            } minimal: {
                Image(systemName: "phone.fill").foregroundStyle(.green).accessibilityLabel("Zooの音声通話")
            }
        }
    }
}
