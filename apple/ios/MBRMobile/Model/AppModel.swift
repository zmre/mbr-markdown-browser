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
        // "Download files on read" was a Settings toggle until 2026-10-15; drop
        // its stored value. The app always runs with Rust's default (dataless
        // files fail fast → Downloading page): turned on, opening a 2.3 GB
        // iCloud vault downloaded the whole vault and the scan had not
        // finished after 5+ minutes. `setMaterializeDatalessFiles` stays
        // exported for tests.
        UserDefaults.standard.removeObject(forKey: "materializeDatalessFiles")
    }

    // MARK: Sessions

    func open(_ route: Route) async throws -> VaultSession {
        if let previous = session {
            self.session = nil
            Self.retire(previous)
        }
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
        self.session = nil
        Self.retire(closing)
        // Warm a page for the next vault.
        self.pool.prewarm()
    }

    /// Stops a session that is leaving the screen, saving a vault's index
    /// first: what was learned since the last save (edits, refreshes) would
    /// otherwise be rescanned on its next open. The flush holds the session,
    /// so its `MbrApp` lives until the save is done.
    private static func retire(_ session: VaultSession) {
        session.close()
        guard !session.isSingleFile else { return }
        Task { await session.flush() }
    }

    /// A file handed to the app (Files "Open in", the share sheet), over
    /// whatever is on screen: the vault list, a vault, another file. The
    /// screen is keyed by its route (`RootView`), so this is a new screen
    /// whose open closes the current session.
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
        Task { await session.refresh(reason: "foreground", askProvider: true) }
    }
}
