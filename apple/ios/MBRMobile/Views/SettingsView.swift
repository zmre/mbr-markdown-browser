import SwiftUI

struct SettingsView: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @AppStorage(AppModel.materializeKey) private var materializeDataless = false
    @AppStorage("showTimingBar") private var showTimingBar = true
    @State private var cache: CacheInfo?
    @State private var clearing = false

    var body: some View {
        Form {
            Section {
                Toggle("Download files on read", isOn: self.$materializeDataless)
                    .onChange(of: self.materializeDataless) { self.model.applyDatalessPolicy() }
            } header: {
                Text("Cloud files")
            } footer: {
                Text("Off (recommended): a note that is not downloaded yet shows a Downloading page, "
                    + "and MBR asks iCloud or the provider for it. On: reading it waits for the download, "
                    + "which can stall a large vault's first open. Applies to the next vault opened; "
                    + "for indexing, from the next launch.")
            }

            Section {
                LabeledContent("Saved indexes", value: self.cache.map { "\($0.vaults)" } ?? "…")
                LabeledContent("Size", value: self.cache.map {
                    ByteCountFormatter.string(fromByteCount: $0.bytes, countStyle: .file)
                } ?? "…")
                if let path = model.session?.app.indexCachePath() {
                    LabeledContent("Open vault", value: (path as NSString).lastPathComponent)
                }
                Button("Clear Cache", role: .destructive) {
                    Task { await self.clear() }
                }
                .disabled(self.clearing)
            } header: {
                Text("Index cache")
            } footer: {
                Text("Each vault's index is saved in the app's cache so it opens without a full scan. "
                    + "Clearing it only costs the next open a scan; your notes are never touched.")
            }

            Section {
                Toggle("Show timings", isOn: self.$showTimingBar)
            } header: {
                Text("Display")
            } footer: {
                Text("Under a vault's title: open · scan · ready · first page, in milliseconds "
                    + "(\"c\": the index came from the cache). The log has the same numbers.")
            }

            Section("Help") {
                NavigationLink("Which folders work") { OnboardingView() }
            }

            Section("About") {
                LabeledContent("Version", value: Self.version)
                LabeledContent("Core", value: "mbr \(Self.marketingVersion)")
            }
        }
        .navigationTitle("Settings")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button("Done") { self.dismiss() }
            }
        }
        .task { self.cache = await CacheInfo.load() }
    }

    private func clear() async {
        self.clearing = true
        await Task.detached(priority: .userInitiated) {
            try? FileManager.default.removeItem(at: AppPaths.vaultCaches)
        }.value
        Log.info("settings: index cache cleared")
        self.cache = await CacheInfo.load()
        self.clearing = false
    }

    static var marketingVersion: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "?"
    }

    static var version: String {
        let build = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "?"
        return "\(marketingVersion) (\(build))"
    }
}

/// The saved index snapshots, measured off the main thread.
private struct CacheInfo {
    let vaults: Int
    let bytes: Int64

    static func load() async -> CacheInfo {
        await Task.detached(priority: .utility) {
            let fileManager = FileManager.default
            let folders = (try? fileManager.contentsOfDirectory(
                at: AppPaths.vaultCaches, includingPropertiesForKeys: nil
            )) ?? []
            var bytes: Int64 = 0
            var vaults = 0
            for folder in folders {
                let files = (try? fileManager.contentsOfDirectory(
                    at: folder, includingPropertiesForKeys: [.fileSizeKey]
                )) ?? []
                if !files.isEmpty { vaults += 1 }
                for file in files {
                    bytes += Int64((try? file.resourceValues(forKeys: [.fileSizeKey]))?.fileSize ?? 0)
                }
            }
            return CacheInfo(vaults: vaults, bytes: bytes)
        }.value
    }
}
