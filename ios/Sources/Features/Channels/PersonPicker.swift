import SwiftUI

struct PersonPicker: View {
    @Environment(ChannelViewModel.self) private var model
    @Environment(SessionController.self) private var session
    @Environment(\.dismiss) private var dismiss
    let selected: (Channel) -> Void
    @State private var search = ""
    @State private var busy = false

    var body: some View {
        NavigationStack {
            List(model.people.filter { !$0.is_bot && $0.id != model.me?.id && (search.isEmpty || $0.display_name.localizedCaseInsensitiveContains(search)) }) { person in
                Button {
                    busy = true
                    Task {
                        _ = await session.perform { let channel = try await model.startDM(person); dismiss(); selected(channel) }
                        busy = false
                    }
                } label: {
                    HStack(spacing: 12) {
                        Avatar(name: person.display_name)
                        VStack(alignment: .leading) { Text(person.display_name); Text("@\(person.user_name)").font(.caption).foregroundStyle(.secondary) }
                    }.foregroundStyle(.primary)
                }.disabled(busy)
            }
            .searchable(text: $search, prompt: "メンバーを検索")
            .navigationTitle("新しいメッセージ").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("閉じる") { dismiss() }.disabled(busy) } }
            .interactiveDismissDisabled(busy)
        }
    }
}
