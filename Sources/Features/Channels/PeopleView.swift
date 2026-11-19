import SwiftUI

struct PeopleView: View {
    let workspace: WorkspaceStore
    @Environment(\.dismiss) private var dismiss
    @State private var search = ""
    @State private var recipients: Set<String> = []
    @State private var submitting = false
    @State private var error: String?

    private var candidates: [Person] {
        workspace.people.values.filter {
            $0.id != workspace.user.id && !$0.is_bot && !$0.is_deactivated &&
            (recipients.contains($0.id) || search.isEmpty || $0.name.localizedStandardContains(search) ||
             $0.user_name.localizedStandardContains(search))
        }.sorted { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    var body: some View {
        NavigationStack {
            AppList {
                if let error { Text(error).foregroundStyle(.red) }
                Section("宛先（9人まで）") {
                    ForEach(candidates) { person in
                        let selected = recipients.contains(person.id)
                        Button {
                            if selected { recipients.remove(person.id) }
                            else { recipients.insert(person.id) }
                        } label: {
                            HStack {
                                PersonRow(person: person).foregroundStyle(.primary)
                                Spacer()
                                Image(systemName: selected ? "checkmark.circle.fill" : "circle")
                            }
                        }
                        .disabled(!selected && recipients.count >= 9)
                        .accessibilityAddTraits(selected ? .isSelected : [])
                        .accessibilityIdentifier("dm-recipient-\(person.id)")
                    }
                }
            }
            .disabled(submitting)
            .appSearch(text: $search, prompt: "名前を検索", identifier: "people") {
                Button("開始") { submitting = true }
                    .disabled(recipients.isEmpty || submitting)
                    .accessibilityIdentifier("start-dm")
            }
            .navigationTitle("新しいDM").navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { AppCloseButton { dismiss() } }
            }
            .task(id: submitting) {
                guard submitting else { return }
                defer { submitting = false }
                do {
                    let ids = recipients.sorted()
                    let channel = try await workspace.client.directMessage(userIDs: ids)
                    try Task.checkCancellation()
                    workspace.addAndOpen(channel, members: ids + [workspace.user.id])
                    dismiss()
                } catch {
                    if !Task.isCancelled { self.error = error.localizedDescription; workspace.report(error) }
                }
            }
        }
    }
}
