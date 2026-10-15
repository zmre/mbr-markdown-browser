import SwiftUI
import UIKit
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
    @State private var searching = false
    /// Counts edge swipes that opened something, to drive their haptic.
    @State private var edgeSwipes = 0

    var body: some View {
        self.content
            // The page's own title: mbr's web header is hidden in the app
            // (app-bridge.ts reports the title, breadcrumbs and actions).
            .navigationTitle(self.pageTitle)
            // The timings ride in the subtitle: a bottom bar would float over
            // the page's own bottom controls (the deck's Show answer, the
            // search modal's footer). Without them, the vault's name.
            .navigationSubtitle(self.subtitle)
            .navigationBarTitleDisplayMode(.inline)
            // Back is web history (HistoryBackButton); the system one would
            // leave the vault from any page.
            .navigationBarBackButtonHidden(self.session != nil)
            .toolbarTitleMenu {
                if let session { BreadcrumbMenu(session: session) }
            }
            .toolbar { self.toolbar }
            // The full-screen editor has its own Cancel/Save header; the bar
            // (Back, the breadcrumb menu, Home) would only offer ways to lose
            // the edits. A playing slide deck wants the whole screen (a
            // landscape deck is squashed under the bar); its ✕ reloads the
            // page, whose page message brings the bar back.
            .toolbar(self.editorOpen || self.slidesPlaying ? .hidden : .automatic, for: .navigationBar)
            .statusBarHidden(self.slidesPlaying)
            .fileImporter(isPresented: self.$picking, allowedContentTypes: [.folder]) { result in
                guard case let .success(folder) = result else { return }
                Task { await self.picked(folder) }
            }
            .fileDialogDefaultDirectory(self.pickerStart)
            .sheet(isPresented: self.$searching) {
                if let session { SearchSheet(session: session) }
            }
            .task(id: self.route) { await self.open() }
            .onDisappear {
                if let session { self.model.close(session) }
            }
    }

    @ViewBuilder
    private var content: some View {
        if let session {
            WebView(session.page)
                // The screen edges open Browse (left) and Info (right), so
                // WebKit's back/forward swipes, which own the same edges, are
                // off for good: both would fire. Back is the bar's button and
                // its long-press history.
                .webViewBackForwardNavigationGestures(.disabled)
                .gesture(EdgeSwipe(edge: .left, isEnabled: self.swipeEnabled(.browse, in: session)) {
                    self.edgeSwiped(.browse, from: "left", in: session)
                })
                .gesture(EdgeSwipe(edge: .right, isEnabled: self.swipeEnabled(.info, in: session)) {
                    self.edgeSwiped(.info, from: "right", in: session)
                })
                .sensoryFeedback(.impact(weight: .light), trigger: self.edgeSwipes)
                // A page no wider than the screen does not rubber-band sideways.
                .scrollBounceBehavior(.basedOnSize, axes: .horizontal)
                .ignoresSafeArea(edges: .bottom)
                .safeAreaInset(edge: .top, spacing: 0) {
                    if let configError = session.configError, !configNoticeDismissed {
                        ConfigNotice(message: configError) { self.configNoticeDismissed = true }
                    }
                }
                // A config that broke (or broke differently) on a refresh is
                // news: show the notice again.
                .onChange(of: session.configError) { self.configNoticeDismissed = false }
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
            ToolbarItem(placement: .topBarLeading) {
                HistoryBackButton(session: session) { self.model.path = [] }
            }
            // One item holding both, so Liquid Glass draws + and ⋯ in one
            // capsule. Not a `ToolbarItemGroup`: beside a long title an
            // iPhone folds a group of two into a single overflow ⋯, which
            // would hide New Note a menu deep.
            ToolbarItem(placement: .topBarTrailing) {
                HStack(spacing: 4) {
                    if !session.isSingleFile, session.web.info.actions.contains(.newNote) {
                        Button(AppAction.newNote.label, systemImage: AppAction.newNote.symbol) {
                            Task {
                                let opened = await AppBridge.open(.newNote, in: session.page)
                                Log.info("bridge: open new -> \(opened)")
                            }
                        }
                    }
                    VaultActionsMenu(
                        session: session,
                        openSearch: { self.searching = true },
                        openSettings: { self.model.showSettings = true },
                        leave: { self.model.path = [] }
                    )
                }
                // In a toolbar item's own view, labels are not icon-only by
                // default.
                .labelStyle(.iconOnly)
            }
            if session.isSingleFile {
                // A capsule of its own: it is about the file, not the page.
                ToolbarSpacer(.fixed, placement: .topBarTrailing)
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

    /// The page's full-screen editor is up (`app-bridge.ts` reports it).
    private var editorOpen: Bool {
        self.session?.web.info.editorOpen ?? false
    }

    /// A slide presentation is playing (`app-bridge.ts` reports it).
    private var slidesPlaying: Bool {
        self.session?.web.info.slidesPlaying ?? false
    }

    /// What the page reported as its title, else the vault's name.
    private var pageTitle: String {
        let reported = self.session?.web.info.title ?? ""
        return reported.isEmpty ? self.title : reported
    }

    private var subtitle: String {
        guard let session else { return "" }
        return self.showTimingBar ? Self.timings(session) : self.title
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

    /// An edge swipe is a shortcut for a ⋯ action: live only where the page
    /// offers it, and never over the editor, a playing deck (Reveal swipes
    /// between slides) or the search sheet.
    private func swipeEnabled(_ action: AppAction, in session: VaultSession) -> Bool {
        session.web.info.actions.contains(action) && !self.editorOpen && !self.slidesPlaying && !self.searching
    }

    // MARK: Actions

    private func edgeSwiped(_ action: AppAction, from edge: String, in session: VaultSession) {
        self.edgeSwipes += 1
        Task {
            let opened = await AppBridge.open(action, in: session.page)
            Log.info("swipe: \(edge) edge -> open \(action.rawValue) -> \(opened)")
        }
    }

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

/// A swipe in from one screen edge: UIKit's `UIScreenEdgePanGestureRecognizer`,
/// so it begins only at the edge (in landscape too, where the edge is the
/// screen's, not the safe area's) and is judged by the system's own edge rules.
/// Attached to the web view as an extra recognizer, not drawn over it: a
/// touch that does not start at the edge, a tap and a vertical scroll all
/// reach the page untouched, which an overlay strip could not promise.
///
/// It fires once per swipe, as soon as the drag is clearly inward and
/// horizontal (or on a short, fast flick at the end); once it fires, UIKit
/// cancels the page's touches, so the page never sees the drag as its own.
private struct EdgeSwipe: UIGestureRecognizerRepresentable {
    /// `.left` or `.right`.
    let edge: UIRectEdge
    let isEnabled: Bool
    let action: () -> Void

    /// Inward distance, in points, at which a drag counts.
    private static let distance: CGFloat = 40
    /// A flick: this inward speed (pt/s) at the end, over at least `flickDistance`.
    private static let flickSpeed: CGFloat = 400
    private static let flickDistance: CGFloat = 16

    final class Coordinator: NSObject, UIGestureRecognizerDelegate {
        var fired = false

        /// Alongside the web view's own recognizers, so it never waits on
        /// (or blocks) scrolling and selection.
        func gestureRecognizer(
            _: UIGestureRecognizer,
            shouldRecognizeSimultaneouslyWith _: UIGestureRecognizer
        ) -> Bool {
            true
        }
    }

    func makeCoordinator(converter _: CoordinateSpaceConverter) -> Coordinator {
        Coordinator()
    }

    func makeUIGestureRecognizer(context: Context) -> UIScreenEdgePanGestureRecognizer {
        let recognizer = UIScreenEdgePanGestureRecognizer()
        recognizer.edges = self.edge
        recognizer.delegate = context.coordinator
        recognizer.isEnabled = self.isEnabled
        return recognizer
    }

    func updateUIGestureRecognizer(_ recognizer: UIScreenEdgePanGestureRecognizer, context _: Context) {
        recognizer.isEnabled = self.isEnabled
    }

    func handleUIGestureRecognizerAction(_ recognizer: UIScreenEdgePanGestureRecognizer, context: Context) {
        let view = recognizer.view
        // Inward is +x from the left edge, -x from the right.
        let sign: CGFloat = self.edge == .right ? -1 : 1
        let inward = recognizer.translation(in: view).x * sign
        let across = abs(recognizer.translation(in: view).y)
        let horizontal = inward > across * 1.5
        switch recognizer.state {
        case .began:
            context.coordinator.fired = false
        case .changed:
            if !context.coordinator.fired, horizontal, inward >= Self.distance {
                context.coordinator.fired = true
                self.action()
            }
        case .ended:
            let speed = recognizer.velocity(in: view).x * sign
            if !context.coordinator.fired, horizontal, inward >= Self.flickDistance, speed >= Self.flickSpeed {
                context.coordinator.fired = true
                self.action()
            }
        default:
            break
        }
    }
}
