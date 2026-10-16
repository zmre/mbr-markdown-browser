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
    /// Why `.mbr/config.toml` is not in use, if it is not. Follows refreshes:
    /// a config broken or fixed mid-session updates it.
    private(set) var configError: String?

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
    /// When a refresh (or the initial indexing's) last changed what pages
    /// show: notes added, changed or removed, or `.mbr/`. A page WebKit
    /// restores from its back/forward cache that was loaded before this is
    /// stale (its navigation came from the old `site.json`), so Back reloads
    /// it (`go(to:)`).
    private var indexChangedAt: ContinuousClock.Instant?
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
            Log.info("ready: waiting for the initial indexing")
            do {
                let report = try await app.waitUntilReady()
                Log.info("ready: fromCache=\(report.fromCache) restored=\(report.restoredFiles) "
                    + "indexed=\(report.indexedFiles) scan=\(report.scanMs)ms ready=\(report.readyMs)ms "
                    + "diff=+\(report.refresh.added)/~\(report.refresh.changed)/-\(report.refresh.removed) "
                    + "cache=\(app.indexCachePath() ?? "none") "
                    + "mbrDownloads=\(report.mbrDownloadsRequested) "
                    + "foldersNotDownloaded=\(report.foldersNotDownloaded)")
                self?.report = report
                // A restored index is answered at once and then corrected by
                // a stat-diff that can take seconds (iCloud listings): the
                // page on screen was drawn from the old one. (After a scan
                // there is nothing to correct: `site.json` waits for it.)
                if report.fromCache {
                    await self?.apply(report.refresh, reason: "initial indexing")
                }
                if report.mbrDownloadsRequested > 0 || report.foldersNotDownloaded > 0 {
                    await self?.refreshUntilDownloaded()
                }
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

    func close() {
        self.watchers.forEach { $0.cancel() }
        self.watchers.removeAll()
        self.web.target.app = nil
        Log.info("close: \(self.title)")
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

// MARK: Freshness

extension VaultSession {
    /// Re-lists the vault (on foreground, or the refresh button) and reloads
    /// the page when what it shows changed: a WebSocket cannot cross a scheme
    /// handler, so the app is the live-reload channel. That includes the
    /// vault's `.mbr/` (config, templates, CSS and component overrides). After
    /// the reload the page reports itself again, so the native bar follows.
    ///
    /// `always` (the Refresh menu item) reloads the page whatever the
    /// refresh found: the reader asked for it, and the index may have changed
    /// before they did (a background refresh, the initial indexing).
    ///
    /// `askProvider` (the Refresh menu item, coming back to the foreground)
    /// first asks a file provider to update the page on screen and its folder
    /// (`askProviderForUpdates`): the stat-diff alone only sees the provider's
    /// local replica, which can lag the cloud indefinitely.
    func refresh(reason: String, always: Bool = false, askProvider: Bool = false) async {
        if askProvider {
            await self.askProviderForUpdates()
        }
        do {
            let summary = try await app.refresh()
            self.lastRefresh = summary
            await self.apply(summary, reason: reason, always: always)
        } catch {
            Log.info("refresh (\(reason)): failed: \(error)")
            if always {
                await self.reload(why: "requested")
            }
        }
    }

    /// Acts on what a refresh found: reloads the page on screen when it
    /// changed, when notes were added or removed (the navigation changed), or
    /// when `.mbr/` changed — or always, when asked.
    private func apply(_ summary: RefreshSummary, reason: String, always: Bool = false) async {
        self.configError = summary.configError
        let shown = self.currentPath
        let pageChanged = shown.map { path in
            summary.changedPages.contains { Self.samePage($0, path) }
        } ?? false
        let navigationChanged = summary.added > 0 || summary.removed > 0
        let mbrChanged = summary.configReloaded || summary.templatesReloaded || summary.assetsChanged
        if summary.added > 0 || summary.changed > 0 || summary.removed > 0 || mbrChanged {
            self.indexChangedAt = .now
        }
        Log.info("refresh (\(reason)): +\(summary.added) ~\(summary.changed) -\(summary.removed) "
            + "evicted=\(summary.evicted) in \(summary.durationMs)ms; changedPages=\(summary.changedPages); "
            + ".mbr: config=\(summary.configReloaded) templates=\(summary.templatesReloaded) "
            + "assets=\(summary.assetsChanged) configError=\(summary.configError ?? "none") "
            + "downloads=\(summary.mbrDownloadsRequested) "
            + "foldersNotDownloaded=\(summary.foldersNotDownloaded); "
            + "shown=\(shown ?? "-")")
        if pageChanged || navigationChanged || mbrChanged || always {
            let why = pageChanged ? "page changed"
                : navigationChanged ? "files added or removed"
                : mbrChanged ? ".mbr/ changed" : "requested"
            await self.reload(why: why)
        }
    }

    /// Reloads the page on screen. The native reload rather than
    /// `location.reload()` from script, so this returns once the page has
    /// reloaded and nothing that follows races the navigation.
    private func reload(why: String) async {
        guard self.page.url != nil else { return }
        Log.info("refresh: reloading \(self.currentPath ?? "page") (\(why))")
        let start = ContinuousClock.now
        do {
            for try await event in self.page.reload() where event == .finished {
                Log.info("refresh: reloaded in \(millis(since: start))ms")
            }
        } catch {
            Log.info("refresh: reload failed: \(error)")
        }
    }
}

// MARK: History

extension VaultSession {
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
    ///
    /// WebKit may restore the page from its back/forward cache — no request
    /// reaches the scheme handler — with the JavaScript state it had,
    /// `site.json` included. If the index changed since that page was loaded,
    /// it is reloaded, so Back never shows navigation older than the index.
    func go(to item: WebPage.BackForwardList.Item) async {
        let document = Self.documentURL(item.url)
        let loadedAt = self.web.target.lastDocumentRequest(document)
        let started = ContinuousClock.now
        do {
            for try await event in self.page.load(item) where event == .finished {
                break
            }
        } catch {
            Log.info("history: \(item.url.absoluteString) failed: \(error)")
            return
        }
        let requested = self.web.target.lastDocumentRequest(document).map { $0 >= started } ?? false
        guard !requested, let changed = indexChangedAt else { return }
        if loadedAt.map({ $0 < changed }) ?? true {
            Log.info("history: \(document.path(percentEncoded: false)) came from the back/forward cache, "
                + "older than the index")
            await self.reload(why: "restored page predates the index")
        }
    }

    /// `url` as the scheme handler saw it: WebKit never sends the fragment.
    private static func documentURL(_ url: URL) -> URL {
        guard var parts = URLComponents(url: url, resolvingAgainstBaseURL: false), parts.fragment != nil else {
            return url
        }
        parts.fragment = nil
        return parts.url ?? url
    }
}

// MARK: Downloads: .mbr/ files and folder listings

extension VaultSession {
    /// Something was in the cloud at open and Rust asked for it; nothing tells
    /// the app when it arrives. Either files in the vault's `.mbr/`
    /// (`user.css`, templates, `config.toml`) — nothing else would ask for
    /// them — or folders whose *listing* was not on the device (an iCloud
    /// Drive folder copied in from another device stays dataless until
    /// something lists it), so their notes were not indexed. Refresh on a
    /// backoff until a refresh reports none outstanding (an arrival reloads
    /// the page) or the schedule runs out; foregrounding and the Refresh
    /// menu item refresh anyway, and ask again.
    private func refreshUntilDownloaded() async {
        for delay in Self.downloadRefreshDelays {
            do {
                try await Task.sleep(for: .seconds(delay))
            } catch {
                return // the session closed
            }
            await self.refresh(reason: "downloads")
            if let summary = lastRefresh, summary.mbrDownloadsRequested == 0, summary.foldersNotDownloaded == 0 {
                return
            }
        }
        Log.info("refresh: .mbr/ files or folders still not downloaded; leaving them to the next refresh")
    }

    private static let downloadRefreshDelays: [Double] = [3, 6, 12, 30, 60]
}
