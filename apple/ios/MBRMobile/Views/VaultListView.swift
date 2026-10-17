import SwiftUI
import UniformTypeIdentifiers

/// The app's root: the vault list, with each vault (or opened file) pushed
/// on top.
struct RootView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model
        NavigationStack(path: $model.path) {
            VaultListView()
                // Keyed by the route: a document handed to the app (or
                // Grant Folder Access, or a re-pick) replaces the path's one
                // route, and SwiftUI would otherwise reuse the screen at that
                // position, with the old route's session in its state.
                .navigationDestination(for: Route.self) { VaultScreen(route: $0).id($0) }
        }
        .sheet(isPresented: $model.showSettings) {
            NavigationStack { SettingsView() }
        }
        .sheet(isPresented: $model.showOnboarding) {
            NavigationStack {
                OnboardingView()
                    .toolbar {
                        ToolbarItem(placement: .confirmationAction) {
                            Button("Done") { model.showOnboarding = false }
                        }
                    }
            }
        }
    }
}

struct VaultListView: View {
    @Environment(AppModel.self) private var model
    @Environment(VaultStore.self) private var store
    @AppStorage("onboardingSeen") private var onboardingSeen = false
    @State private var picking = false
    @State private var renaming: Vault?
    @State private var newName = ""

    var body: some View {
        List {
            Section {
                ForEach(self.store.vaults) { vault in
                    NavigationLink(value: Route.vault(vault.id)) {
                        Label(vault.name, systemImage: vault.isContainer ? "book.closed" : "folder")
                    }
                    .contextMenu {
                        Button("Rename", systemImage: "pencil") { self.beginRename(vault) }
                        Button("Remove", systemImage: "trash", role: .destructive) { self.store.remove(vault.id) }
                    }
                    .swipeActions {
                        Button("Remove", systemImage: "trash", role: .destructive) { self.store.remove(vault.id) }
                        Button("Rename", systemImage: "pencil") { self.beginRename(vault) }
                            .tint(.indigo)
                    }
                }
            } header: {
                Text("Vaults")
            } footer: {
                VStack(alignment: .leading, spacing: 6) {
                    Text("Add a folder of markdown notes with +. MBR reads it where it is; nothing is copied. "
                        + "Removing a vault here never deletes the folder.")
                    Button("Which folders work?") { self.model.showOnboarding = true }
                        .font(.footnote)
                }
            }
            if let error = store.lastError {
                Section("Last error") {
                    Text(error).foregroundStyle(.red)
                    Button("Dismiss") { self.store.lastError = nil }
                }
            }
        }
        .navigationTitle("MBR")
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                Button("Settings", systemImage: "gear") { self.model.showSettings = true }
            }
            ToolbarItem(placement: .primaryAction) {
                Button("Add Folder", systemImage: "folder.badge.plus") { self.picking = true }
            }
        }
        .fileImporter(isPresented: self.$picking, allowedContentTypes: [.folder]) { result in
            switch result {
            case let .success(url): Task { await self.store.add(folder: url) }
            case let .failure(error): self.store.lastError = error.localizedDescription
            }
        }
        .alert("Rename vault", isPresented: self.renamingBinding) {
            TextField("Name", text: self.$newName)
            Button("Cancel", role: .cancel) {}
            Button("Rename") {
                if let renaming { self.store.rename(renaming.id, to: self.newName) }
            }
        } message: {
            Text("Only the name shown in MBR changes; the folder keeps its name.")
        }
        .onAppear {
            if !self.onboardingSeen, !SelfTest.isActive {
                self.onboardingSeen = true
                self.model.showOnboarding = true
            }
        }
    }

    private var renamingBinding: Binding<Bool> {
        Binding(get: { self.renaming != nil }, set: { if !$0 { self.renaming = nil } })
    }

    private func beginRename(_ vault: Vault) {
        self.newName = vault.name
        self.renaming = vault
    }
}
