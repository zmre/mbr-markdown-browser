import SwiftUI
import UniformTypeIdentifiers
import WebKit

/// One open vault or file: the web view, plus the native bits around it.
struct VaultScreen: View {
    let route: Route
    @Environment(AppModel.self) private var model
    @Environment(VaultStore.self) private var store
    @AppStorage("showTimingBar") private var showTimingBar = true
    @State private var session: VaultSession?
    @State private var error: String?
    @State private var picking = false
    @State private var configNoticeDismissed = false

    var body: some View {
        self.content
            .navigationTitle(self.title)
            // The timings ride in the subtitle: a bottom bar would float over
            // the page's own bottom controls (the deck's Show answer, the
            // search modal's footer).
            .navigationSubtitle(self.showTimingBar ? self.session.map(Self.timings) ?? "" : "")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { self.toolbar }
            .fileImporter(isPresented: self.$picking, allowedContentTypes: [.folder]) { result in
                guard case let .success(folder) = result else { return }
                Task { await self.picked(folder) }
            }
            .fileDialogDefaultDirectory(self.pickerStart)
            .task(id: self.route) { await self.open() }
            .onDisappear {
                if let session { self.model.close(session) }
            }
    }

    @ViewBuilder
    private var content: some View {
        if let session {
            WebView(session.page)
                .webViewBackForwardNavigationGestures(.enabled)
                .ignoresSafeArea(edges: .bottom)
                .safeAreaInset(edge: .top, spacing: 0) {
                    if let configError = session.configError, !configNoticeDismissed {
                        ConfigNotice(message: configError) { self.configNoticeDismissed = true }
                    }
                }
                .alert(
                    session.web.dialogs.pending?.isConfirm == true ? "Confirm" : "MBR",
                    isPresented: self.dialogBinding(session.web.dialogs),
                    presenting: session.web.dialogs.pending
                ) { pending in
                    if pending.isConfirm {
                        Button("Cancel", role: .cancel) { session.web.dialogs.answer(false) }
                    }
                    Button("OK") { session.web.dialogs.answer(true) }
                } message: { pending in
                    Text(pending.message)
                }
        } else if let error {
            ContentUnavailableView {
                Label("Could not open \(self.title)", systemImage: "exclamationmark.triangle")
            } description: {
                Text(error)
            } actions: {
                if case .vault = self.route {
                    Button("Re-pick Folder…") { self.picking = true }
                        .buttonStyle(.borderedProminent)
                }
            }
        } else {
            ProgressView("Opening \(self.title)…")
        }
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        if let session {
            ToolbarItem(placement: .primaryAction) {
                Menu("More", systemImage: "ellipsis.circle") {
                    Button("Refresh", systemImage: "arrow.clockwise") {
                        Task { await session.refresh(reason: "button") }
                    }
                    Button("Home", systemImage: "house") {
                        Task { await session.load(MbrSchemeHandler.url(forPath: session.app.homeUrl()), label: "home") }
                    }
                    Button("Settings", systemImage: "gear") { self.model.showSettings = true }
                }
            }
            if session.isSingleFile {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Grant Folder Access…", systemImage: "folder.badge.plus") { self.picking = true }
                        .help("Open the folder around this file, so its links and images work")
                }
            }
        }
    }

    /// `open · scan · ready · page`, for device measurements.
    private static func timings(_ session: VaultSession) -> String {
        let scan = session.report.map { "\($0.scanMs)" } ?? "…"
        let ready = session.report.map { "\($0.readyMs)\($0.fromCache ? "c" : "")" } ?? "…"
        let page = session.firstPageMs.map { "\($0)" } ?? "…"
        // open · scan · ready (c = from the index cache) · first page, in ms.
        return "\(session.openMs) · \(scan) · \(ready) · \(page) ms"
    }

    private var title: String {
        switch self.route {
        case let .vault(id, _): self.store.vault(id)?.name ?? "Vault"
        case let .file(url): url.lastPathComponent
        }
    }

    /// Where the folder picker starts: the opened file's folder, for "Grant
    /// folder access…".
    private var pickerStart: URL? {
        if case let .file(url) = route { return url.deletingLastPathComponent() }
        return nil
    }

    private func dialogBinding(_ dialogs: DialogModel) -> Binding<Bool> {
        Binding(get: { dialogs.pending != nil }, set: { if !$0 { dialogs.answer(false) } })
    }

    // MARK: Actions

    private func open() async {
        guard self.session == nil else { return }
        self.error = nil
        do {
            let opened = try await model.open(self.route)
            self.session = opened
            var start: String?
            if case let .vault(_, path) = route { start = path }
            await opened.start(at: start)
            await SelfTest.sessionStarted(opened, model: self.model)
        } catch {
            self.error = error.localizedDescription
            Log.info("open failed: \(error)")
        }
    }

    /// A folder came back from the picker: either the recovery for a vault
    /// whose bookmark broke, or "Grant folder access…" for an opened file,
    /// which becomes a vault opened at that file.
    private func picked(_ folder: URL) async {
        switch self.route {
        case let .vault(id, start):
            if await self.store.repick(id, folder: folder) {
                self.session = nil
                self.model.path = [.vault(id, start: start)]
            }
        case let .file(file):
            guard let vault = await store.add(folder: folder) else { return }
            let start = Self.pagePath(of: file, in: folder)
            Log.info("grant: \(folder.path(percentEncoded: false)) is now a vault; opening at \(start ?? "/")")
            self.model.path = [.vault(vault.id, start: start)]
        }
    }

    /// The URL path of `file` inside `folder` (`/notes/a%20b.md`, which mbr
    /// redirects to the page), or `nil` when it is not inside.
    static func pagePath(of file: URL, in folder: URL) -> String? {
        let filePath = file.resolvingSymlinksInPath().standardizedFileURL.pathComponents
        let folderPath = folder.resolvingSymlinksInPath().standardizedFileURL.pathComponents
        guard filePath.count > folderPath.count, Array(filePath.prefix(folderPath.count)) == folderPath else {
            return nil
        }
        let relative = filePath.dropFirst(folderPath.count)
            .map { $0.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? $0 }
        return "/" + relative.joined(separator: "/")
    }
}

/// "This vault's `.mbr/config.toml` was not used", above the page.
private struct ConfigNotice: View {
    let message: String
    let dismiss: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.yellow)
            VStack(alignment: .leading, spacing: 2) {
                Text("Configuration ignored").font(.footnote.bold())
                Text("This vault's .mbr/config.toml could not be read, so MBR opened it with the defaults. "
                    + self.message)
                    .font(.caption)
                    .lineLimit(4)
            }
            Spacer(minLength: 0)
            Button("Dismiss", systemImage: "xmark", action: self.dismiss)
                .labelStyle(.iconOnly)
                .font(.footnote)
        }
        .padding(10)
        .background(.yellow.opacity(0.15))
        .accessibilityElement(children: .combine)
    }
}
