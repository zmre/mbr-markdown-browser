import Foundation
import Network

/// Whether the device has a network path (`NWPathMonitor`), pushed to every
/// live page as `html.mbr-offline`.
///
/// Only the Downloading page reads the class (error.html's offline variant:
/// "You're offline." instead of a progress bar that would spin forever). The
/// page reloads itself every 2 s (`Refresh`), so the class is set at document
/// start by the bridge's user script, which is replaced on every change, and
/// also toggled in the page on screen so it switches without waiting for the
/// next reload. Reloads continue either way: the note loads by itself once
/// the connection returns.
@MainActor
final class NetworkStatus {
    static let shared = NetworkStatus()

    /// What the app shows: the self-test's forced value, else the monitor's.
    var isOffline: Bool {
        self.forced ?? self.measured
    }

    /// Optimistic until the monitor's first report (which comes at once).
    private var measured = false
    private var forced: Bool?
    private let monitor = NWPathMonitor()
    private var pages: [WeakPage] = []

    private struct WeakPage {
        weak var page: MbrWebPage?
    }

    private init() {
        self.monitor.pathUpdateHandler = { path in
            let offline = path.status != .satisfied
            Task { @MainActor in
                NetworkStatus.shared.update { $0.measured = offline }
            }
        }
        self.monitor.start(queue: DispatchQueue(label: "com.zmre.mbr.ios.network"))
    }

    /// Keeps `page` told about changes, for as long as it lives.
    func register(_ page: MbrWebPage) {
        self.pages.removeAll { $0.page == nil }
        self.pages.append(WeakPage(page: page))
    }

    /// Self-test only: pretends the device is offline (`true`), online
    /// (`false`), or goes back to the monitor (`nil`).
    func force(_ offline: Bool?) {
        self.update { $0.forced = offline }
    }

    private func update(_ change: (NetworkStatus) -> Void) {
        let before = self.isOffline
        change(self)
        let now = self.isOffline
        guard now != before else { return }
        Log.info("network: \(now ? "offline" : "online")\(self.forced == nil ? "" : " (forced)")")
        self.pages.removeAll { $0.page == nil }
        for entry in self.pages {
            entry.page?.setOffline(now)
        }
    }
}
