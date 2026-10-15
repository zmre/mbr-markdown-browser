import Foundation
import Observation
import UIKit

/// A screen on the navigation stack.
enum Route: Hashable {
    /// A vault from the list, optionally opening at a URL path other than its
    /// home (after "Grant folder access…", the file that was open).
    case vault(UUID, start: String? = nil)
    /// One markdown file opened in place.
    case file(URL)
}

/// App-wide state: the vault list, navigation, the open session, and the app
/// lifecycle hooks that keep the open vault fresh.
@MainActor
@Observable
final class AppModel {
    let store = VaultStore()
    let pool = WebPagePool()
    let coordinator = FileCoordinatorBridge()

    var path: [Route] = []
    var showSettings = false
    var showOnboarding = false
    /// The session on screen. One at a time: opening another closes it.
    private(set) var session: VaultSession?
    private var wasInBackground = false

    init() {
        self.applyDatalessPolicy()
    }

    // MARK: Sessions

    func open(_ route: Route) async throws -> VaultSession {
        self.session?.close()
        self.session = nil
        let opened: VaultSession
        switch route {
        case let .vault(id, _):
            guard let vault = store.vault(id) else { throw CocoaError(.fileNoSuchFile) }
            opened = try await VaultSession.open(
                vault,
                store: self.store,
                pool: self.pool,
                coordinator: self.coordinator
            )
        case let .file(url):
            opened = try await VaultSession.openFile(url, pool: self.pool, coordinator: self.coordinator)
        }
        self.session = opened
        return opened
    }

    func close(_ closing: VaultSession) {
        guard self.session === closing else { return }
        closing.close()
        self.session = nil
        // Warm a page for the next vault.
        self.pool.prewarm()
    }

    /// A file handed to the app (Files "Open in", the share sheet).
    func openDocument(_ url: URL) {
        Log.info("document: open \(url.path(percentEncoded: false))")
        self.path = [.file(url)]
    }

    // MARK: Lifecycle

    /// Saves the index while iOS still gives the app time to: suspension
    /// follows within seconds.
    func didEnterBackground() {
        self.wasInBackground = true
        guard let session else { return }
        let application = UIApplication.shared
        var task = UIBackgroundTaskIdentifier.invalid
        task = application.beginBackgroundTask(withName: "mbr.flushIndex") {
            application.endBackgroundTask(task)
            task = .invalid
        }
        Task {
            await session.flush()
            if task != .invalid {
                application.endBackgroundTask(task)
                task = .invalid
            }
        }
    }

    /// Back in the foreground: re-list the vault, which is the only way to
    /// learn about changes made while the app was away.
    func didBecomeActive() {
        guard self.wasInBackground else { return }
        self.wasInBackground = false
        guard let session else { return }
        Task { await session.refresh(reason: "foreground") }
    }

    // MARK: Settings

    static let materializeKey = "materializeDatalessFiles"

    /// Whether mbr's threads wait for not-downloaded files (the debug toggle).
    /// Off by default: reads of dataless files fail fast and the page says
    /// "Downloading". Takes effect for the next vault opened (and for the
    /// scan's thread pool, from the next launch).
    func applyDatalessPolicy() {
        let materialize = UserDefaults.standard.bool(forKey: Self.materializeKey)
        setMaterializeDatalessFiles(materialize: materialize)
        Log.info("iopolicy: materialize dataless files = \(materializesDatalessFiles())")
    }
}
