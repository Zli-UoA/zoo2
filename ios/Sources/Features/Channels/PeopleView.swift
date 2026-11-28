import SwiftUI

struct PeopleView: View {
    let workspace: WorkspaceStore
    var draftContent: String? = nil
    @Environment(\.dismiss) private var dismiss
    @State private var search = ""
    @State private var recipients: Set<String> = []
    @State private var submitting = false
    @State private var encrypted = false
    @State private var error: String?

    private var eligibleIDs: Set<String> {
        Set(workspace.people.values.filter { $0.id != workspace.user.id && !$0.is_bot && !$0.is_deactivated }.map(\.id))
    }

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
                if draftContent == nil, workspace.e2ee != nil {
                    Section {
                        Toggle("暗号化", isOn: $encrypted).accessibilityIdentifier("dm-encrypted")
                    } footer: { Text("暗号化したDMは、あとから暗号化をやめられません。サーバーにも、運営にも本文は読めません。") }
                }
                Section(draftContent == nil ? "宛先（9人まで）" : "宛先") {
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
                        .disabled(!selected && recipients.count >= (draftContent == nil ? 9 : 1))
                        .accessibilityAddTraits(selected ? .isSelected : [])
                        .accessibilityIdentifier("dm-recipient-\(person.id)")
                    }
                }
            }
            .disabled(submitting)
            .onChange(of: eligibleIDs, initial: true) { _, ids in recipients.formIntersection(ids) }
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
                    recipients.formIntersection(eligibleIDs)
                    let ids = recipients.sorted()
                    guard !ids.isEmpty else { throw ZooError.auth("宛先を選び直してください。") }
                    if let draftContent {
                        let actions = ShortcutActions(workspace: workspace)
                        let targets = try actions.recipients(in: Array(workspace.people.values)).filter { recipients.contains($0.personID) }
                        guard ids.count == 1, targets.count == 1, let target = targets.first else {
                            throw ZooError.auth("宛先を1人選び直してください。")
                        }
                        try await actions.draft(draftContent, to: target)
                    } else {
                        let channel = try await workspace.client.directMessage(userIDs: ids, encrypted: encrypted)
                        try Task.checkCancellation()
                        workspace.addAndOpen(channel, members: ids + [workspace.user.id])
                    }
                    dismiss()
                } catch {
                    if !Task.isCancelled { self.error = error.localizedDescription; workspace.report(error) }
                }
            }
        }
    }
}
