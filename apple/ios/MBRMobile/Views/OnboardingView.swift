import SwiftUI

/// Which storage providers can hand MBR a folder, and how to keep a vault
/// usable offline. Shown on first launch and from Settings.
struct OnboardingView: View {
    var body: some View {
        List {
            Section {
                Text("MBR reads a folder of markdown files where it already is: in iCloud Drive, on this "
                    + "device, or in another app's storage. Tap + on the vault list, pick the folder, then Open.")
            }

            Section("Folders work in") {
                self.provider("iCloud Drive", "icloud", "Including your Mac's ~/Notes if it syncs there.")
                self.provider("On My iPhone / iPad", "iphone", "Folders on this device.")
                self.provider("Working Copy", "arrow.triangle.branch", "Git repositories cloned in Working Copy.")
                self.provider("Dropbox", "shippingbox", "Changes made elsewhere appear once Dropbox has synced them.")
            }

            Section {
                self.provider("Google Drive", "xmark.circle", "Grants single files only.")
                self.provider("OneDrive", "xmark.circle", "Grants single files only.")
                self.provider("Sync.com", "xmark.circle", "Grants single files only.")
            } header: {
                Text("Single files only")
            } footer: {
                Text("These providers cannot share a whole folder with another app. You can still open one "
                    + "note at a time from the Files app (Share → Open in MBR); links to other notes and "
                    + "images beside it will not resolve.")
            }

            Section {
                Text("Cloud providers may keep only the names of files on the device until they are opened. "
                    + "MBR shows a Downloading page for those and asks for them, but for reading offline "
                    + "(and the fastest start), keep the folder downloaded:")
                Label(
                    "In Files, long-press the vault folder and choose Keep Downloaded.",
                    systemImage: "arrow.down.circle"
                )
            } header: {
                Text("Tip: Keep Downloaded")
            }

            Section("Editing") {
                Text("Ticking a task and reviewing flashcards write to your notes, coordinated with the "
                    + "provider. If a note changed elsewhere since it was shown, MBR refuses the write "
                    + "rather than overwrite it; refresh and try again.")
            }
        }
        .navigationTitle("Which folders work")
        .navigationBarTitleDisplayMode(.inline)
    }

    private func provider(_ name: String, _ symbol: String, _ detail: String) -> some View {
        Label {
            VStack(alignment: .leading) {
                Text(name)
                Text(detail).font(.caption).foregroundStyle(.secondary)
            }
        } icon: {
            Image(systemName: symbol)
        }
    }
}
