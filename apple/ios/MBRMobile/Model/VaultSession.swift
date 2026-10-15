import Foundation
import Observation
import WebKit

/// One open vault (or single file): mbr's in-process server for it, and the
/// web page showing it.
@MainActor
@Observable
final class VaultSession {
    enum Kind: Equatable {
        case vault(UUID)
        case file
    }

    let kind: Kind
    let title: String
    let app: MbrApp
    let web: MbrWebPage
    /// Held for the session's lifetime: dropping it ends the security-scoped
    /// access the `MbrApp` reads through.
    let access: FolderAccess
    /// What the vault's writes go through.
    let coordinator: FileCoordinatorBridge

    // Timings for the status bar and the log.
    let openMs: Int
    private(set) var report: OpenReport?
    private(set) var firstPageMs: Int?
    private(set) var lastRefresh: RefreshSummary?
    let configError: String?

    var page: WebPage {
        self.web.page
    }

    var isSingleFile: Bool {
        self.kind == .file
    }

    private var watchers: [Task<Void, Never>] = []
    /// Whether WebKit was seen to re-request a 503 page by itself (`Refresh`).
    private(set) var webKitHonoursRefresh: Bool?
    /// Back-list entries that predate this vault's first page.
    private var historyBase = 0
    private var retries: [URL: Int] = [:]
    private static let maxRetries = 150

    // MARK: Opening

    /// Opens a vault from the list. Everything slow (resolving the bookmark,
    /// `MbrApp.open` reading config and templates) runs off the main thread.
    static func open(
        _ vault: Vault,
        store: VaultStore,
        pool: WebPagePool,
        coordinator: FileCoordinatorBridge
    ) async throws -> VaultSession {
        let access = try await store.access(vault)
        let root = access.path
        let cacheDir = AppPaths.cacheFolder(for: vault.id).path(percentEncoded: false)
        let start = ContinuousClock.now
        // The vault's id is created with its bookmark and survives the
        // bookmark being refreshed or re-picked, so it is the snapshot's
        // identity: a folder whose path moves still finds its index.
        let app = try await Task.detached(priority: .userInitiated) {
            try MbrApp.open(
                rootPath: root,
                cacheDir: cacheDir,
                vaultId: vault.id.uuidString,
                coordinator: coordinator
            )
        }.value
        let openMs = millis(since: start)
        Log.info("open: MbrApp.open took \(openMs)ms for \(root)")
        return VaultSession(
            kind: .vault(vault.id),
            title: vault.name,
            app: app,
            access: access,
            openMs: openMs,
            pool: pool,
            coordinator: coordinator
        )
    }

    /// Opens one markdown file in place (Files' "Open in", the share sheet).
    static func openFile(_ url: URL, pool: WebPagePool, coordinator: FileCoordinatorBridge) async throws
        -> VaultSession {
        let start = ContinuousClock.now
        let access = await Task.detached(priority: .userInitiated) {
            FolderAccess(url: url, scoped: url.startAccessingSecurityScopedResource())
        }.value
        let path = access.path
        let app = try await Task.detached(priority: .userInitiated) {
            try MbrApp.openFile(filePath: path, coordinator: coordinator)
        }.value
        let openMs = millis(since: start)
        Log.info("open: MbrApp.openFile took \(openMs)ms for \(path)")
        return VaultSession(
            kind: .file,
            title: url.lastPathComponent,
            app: app,
            access: access,
            openMs: openMs,
            pool: pool,
            coordinator: coordinator
        )
    }

    private init(
        kind: Kind,
        title: String,
        app: MbrApp,
        access: FolderAccess,
        openMs: Int,
        pool: WebPagePool,
        coordinator: FileCoordinatorBridge
    ) {
        self.kind = kind
        self.coordinator = coordinator
        self.title = title
        self.app = app
        self.access = access
        self.openMs = openMs
        configError = app.configError()
        self.web = pool.take()
        self.web.target.app = app
        if let configError {
            Log.info("config: .mbr/config.toml not used: \(configError)")
        }
    }

    /// Loads the first page (`start`, or the vault's home) and times indexing
    /// in the background.
    func start(at start: String? = nil) async {
        let app = app
        self.watchers.append(Task { [weak self] in
            do {
                let report = try await app.waitUntilReady()
                Log.info("ready: fromCache=\(report.fromCache) restored=\(report.restoredFiles) "
                    + "indexed=\(report.indexedFiles) scan=\(report.scanMs)ms ready=\(report.readyMs)ms "
                    + "diff=+\(report.refresh.added)/~\(report.refresh.changed)/-\(report.refresh.removed) "
                    + "cache=\(app.indexCachePath() ?? "none")")
                self?.report = report
            } catch {
                Log.info("ready: failed: \(error)")
            }
        })
        self.watchers.append(Task { [weak self] in await self?.watchNavigations() })
        self.web.target.onRetryableDocument { [weak self] url, seconds in
            Task { @MainActor in await self?.retryWhenDownloaded(url, after: seconds) }
        }
        let url = MbrSchemeHandler.url(forPath: start ?? app.homeUrl())
        self.firstPageMs = await self.load(url, label: "first page")
        // Whatever is behind the first page (the prewarm document) is not
        // this vault's history: Back from here leaves the vault.
        self.historyBase = self.page.backForwardList.backList.count
    }

    /// Loads `url` and returns the milliseconds until navigation finished.
    @discardableResult
    func load(_ url: URL, label: String) async -> Int? {
        let start = ContinuousClock.now
        do {
            for try await event in self.page.load(URLRequest(url: url)) where event == .finished {
                let ms = millis(since: start)
                Log.info("\(label): \(url.absoluteString) finished in \(ms)ms")
                return ms
            }
        } catch {
            Log.info("\(label): \(url.absoluteString) failed: \(error)")
        }
        return nil
    }

    // MARK: History

    /// Whether Back goes to an earlier page of this vault (rather than out to
    /// the vault list).
    var canGoBack: Bool {
        self.page.backForwardList.backList.count > self.historyBase
    }

    /// This vault's earlier pages, most recent first: the Back button's
    /// long-press menu.
    var recentPages: [WebPage.BackForwardList.Item] {
        Array(self.page.backForwardList.backList.dropFirst(self.historyBase).reversed())
    }

    /// Web-history back one page.
    func goBack() async {
        guard self.canGoBack, let item = self.page.backForwardList.backList.last else { return }
        await self.go(to: item)
    }

    /// Jumps to an entry of the back list.
    func go(to item: WebPage.BackForwardList.Item) async {
        do {
            for try await event in self.page.load(item) where event == .finished {
                break
            }
        } catch {
            Log.info("history: \(item.url.absoluteString) failed: \(error)")
        }
    }

    func close() {
        self.watchers.forEach { $0.cancel() }
        self.watchers.removeAll()
        self.web.target.app = nil
        Log.info("close: \(self.title)")
    }

    // MARK: Freshness

    /// Re-lists the vault (on foreground, or the refresh button) and reloads
    /// the page when what it shows changed: a WebSocket cannot cross a scheme
    /// handler, so the app is the live-reload channel.
    func refresh(reason: String) async {
        do {
            let summary = try await app.refresh()
            self.lastRefresh = summary
            let shown = self.currentPath
            let pageChanged = shown.map { path in
                summary.changedPages.contains { Self.samePage($0, path) }
            } ?? false
            let navigationChanged = summary.added > 0 || summary.removed > 0
            Log.info("refresh (\(reason)): +\(summary.added) ~\(summary.changed) -\(summary.removed) "
                + "evicted=\(summary.evicted) in \(summary.durationMs)ms; changedPages=\(summary.changedPages); "
                + "shown=\(shown ?? "-")")
            if pageChanged || navigationChanged {
                Log.info("refresh: reloading \(shown ?? "page") ("
                    + (pageChanged ? "page changed" : "files added or removed") + ")")
                // The native reload rather than `location.reload()` from
                // script, so this returns once the page has reloaded and
                // nothing that follows races the navigation.
                let start = ContinuousClock.now
                do {
                    for try await event in self.page.reload() where event == .finished {
                        Log.info("refresh: reloaded in \(millis(since: start))ms")
                    }
                } catch {
                    Log.info("refresh: reload failed: \(error)")
                }
            }
        } catch {
            Log.info("refresh (\(reason)): failed: \(error)")
        }
    }

    /// Saves the index snapshot (as the app goes to the background).
    func flush() async {
        let start = ContinuousClock.now
        do {
            try await self.app.flushIndex()
            Log.info("flush: index saved in \(millis(since: start))ms")
        } catch {
            Log.info("flush: failed: \(error)")
        }
    }

    /// The URL path of the page on screen, percent-decoded (as Rust reports
    /// `changedPages`).
    var currentPath: String? {
        self.page.url?.path(percentEncoded: false)
    }

    static func samePage(_ lhs: String, _ rhs: String) -> Bool {
        func trimmed(_ path: String) -> String {
            path.hasSuffix("/") && path.count > 1 ? String(path.dropLast()) : path
        }
        return trimmed(lhs) == trimmed(rhs)
    }

    // MARK: Recovery

    /// Follows every navigation of the page. WebKit kills a WebContent
    /// process under memory pressure (and reclaims backgrounded ones); the
    /// page would stay blank, so it is reloaded.
    private func watchNavigations() async {
        // The page's own URL is gone once its process has died.
        var shown: URL?
        while !Task.isCancelled {
            do {
                for try await event in self.page.navigations where event == .finished {
                    shown = page.url
                    Log.info("page: finished \(page.url?.absoluteString ?? "?")")
                }
                return
            } catch WebPage.NavigationError.webContentProcessTerminated {
                let url = self.page.url ?? shown ?? MbrSchemeHandler.url(forPath: self.app.homeUrl())
                Log.info("page: web content process terminated; reloading \(url.absoluteString)")
                await self.load(url, label: "reload after crash")
            } catch {
                // A failed navigation ends this sequence; keep watching.
                Log.info("page: navigation failed: \(error)")
            }
        }
    }

    /// A page answered `503 Downloading` (`Retry-After`, `Refresh`). If WebKit
    /// does not re-request it by itself once the delay has passed, reload it
    /// from here, while it is still the page on screen.
    private func retryWhenDownloaded(_ url: URL, after seconds: Int) async {
        let answered = ContinuousClock.now
        try? await Task.sleep(for: .seconds(Double(seconds) + 1.5))
        if let last = web.target.lastDocumentRequest(url), last > answered {
            if self.webKitHonoursRefresh != true {
                Log.info("downloading: WebKit re-requested \(url.path()) by itself (Refresh honoured)")
            }
            self.webKitHonoursRefresh = true
            return
        }
        guard self.page.url == url else { return }
        let attempt = self.retries[url, default: 0] + 1
        self.retries[url] = attempt
        guard attempt <= Self.maxRetries else {
            Log.info("downloading: giving up on \(url.path()) after \(Self.maxRetries) retries")
            return
        }
        if self.webKitHonoursRefresh == nil {
            Log.info("downloading: WebKit did not re-request \(url.path()) (Refresh ignored); retrying from Swift")
        }
        self.webKitHonoursRefresh = false
        Log.info("downloading: retry \(attempt) of \(url.path())")
        await self.load(url, label: "downloading retry")
    }
}
