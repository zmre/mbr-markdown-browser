import Foundation
import Observation
import UIKit
import WebKit

/// A `WebPage` wired for mbr: the `mbr://` scheme handler (through a
/// `SchemeTarget` the session fills in once its vault is open), link policy,
/// JavaScript dialogs, and the app bridge (embedded mode, and the page's
/// title, breadcrumbs and actions for the navigation bar).
@MainActor
final class MbrWebPage {
    let page: WebPage
    let target: SchemeTarget
    let dialogs: DialogModel
    let info: PageInfo
    private let decider: LinkPolicy
    /// The page's scripts, kept to replace the document-start script when the
    /// network state changes (`NetworkStatus`).
    private let contentController: WKUserContentController

    init() {
        let target = SchemeTarget()
        let dialogs = DialogModel()
        let decider = LinkPolicy()
        var configuration = WebPage.Configuration()
        configuration.urlSchemeHandlers[URLScheme(MbrSchemeHandler.scheme)!] = MbrSchemeHandler(target: target)
        let info = PageInfo()
        AppBridge.install(in: &configuration, info: info, offline: NetworkStatus.shared.isOffline)
        self.contentController = configuration.userContentController
        // Videos play in the page rather than jumping to fullscreen.
        configuration.mediaPlaybackBehavior = .allowsInlinePlayback
        self.page = WebPage(configuration: configuration, navigationDecider: decider, dialogPresenter: dialogs)
        #if DEBUG
            self.page.isInspectable = true
        #endif
        decider.page = self.page
        self.target = target
        self.dialogs = dialogs
        self.decider = decider
        self.info = info
        NetworkStatus.shared.register(self)
    }

    /// Shows or hides the Downloading page's offline variant, now and on
    /// every later load.
    func setOffline(_ offline: Bool) {
        Task {
            await AppBridge.setOffline(offline, controller: self.contentController, page: self.page)
        }
    }
}

/// Keeps one web page warm so opening a vault does not wait for WebKit to
/// spawn its WebContent process (280–580 ms cold, research doc §8).
///
/// The warm page loads an empty document whose base URL is `mbr://localhost/`
/// rather than `about:blank`, so the vault's first real load is same-site and
/// WebKit has no reason to swap processes.
@MainActor
final class WebPagePool {
    private var warm: MbrWebPage?

    /// Starts warming a page, unless one is warm already.
    func prewarm() {
        guard self.warm == nil else { return }
        let start = ContinuousClock.now
        let candidate = MbrWebPage()
        self.warm = candidate
        Task {
            do {
                for try await event in candidate.page.load(
                    html: "<!doctype html><title></title>", baseURL: MbrSchemeHandler.origin
                ) where event == .finished {
                    Log.info("prewarm: web page ready in \(millis(since: start))ms")
                }
            } catch {
                Log.info("prewarm: failed: \(error)")
            }
        }
    }

    /// The warm page, or a new one if none is warm.
    func take() -> MbrWebPage {
        if let warm {
            self.warm = nil
            return warm
        }
        Log.info("prewarm: no warm page; creating one")
        return MbrWebPage()
    }
}

/// Where a navigation is allowed to go. mbr pages stay in the web view; a
/// click on an external link opens the system's handler (Safari, Mail, …),
/// so the reader never strands the app on a web page. Subframes (a YouTube
/// embed) load in place.
@MainActor
private final class LinkPolicy: WebPage.NavigationDeciding {
    weak var page: WebPage?

    func decidePolicy(
        for action: WebPage.NavigationAction,
        preferences _: inout WebPage.NavigationPreferences
    ) async -> WKNavigationActionPolicy {
        guard let url = action.request.url, let scheme = url.scheme?.lowercased() else { return .allow }
        switch scheme {
        case MbrSchemeHandler.scheme:
            // `target="_blank"` (no target frame): open it here, as there are
            // no tabs to open it in.
            if action.target == nil, let page {
                Task {
                    do {
                        for try await event in page.load(URLRequest(url: url)) where event == .finished {
                            break
                        }
                    } catch {
                        Log.info("link: \(url.absoluteString) failed: \(error)")
                    }
                }
                return .cancel
            }
            return .allow
        case "about", "data", "blob":
            return .allow
        default:
            if let target = action.target, !target.isMainFrame {
                return .allow
            }
            Log.info("link: opening \(url.absoluteString) outside the app")
            await UIApplication.shared.open(url)
            return .cancel
        }
    }
}

/// `alert()` and `confirm()` from the page (deleting a review note, a task
/// write's error), shown as SwiftUI alerts by `VaultScreen`.
@MainActor
@Observable
final class DialogModel: WebPage.DialogPresenting {
    struct Pending: Identifiable {
        let id = UUID()
        let message: String
        let isConfirm: Bool
        fileprivate let resume: (Bool) -> Void
    }

    private(set) var pending: Pending?

    func answer(_ ok: Bool) {
        let current = self.pending
        self.pending = nil
        current?.resume(ok)
    }

    func handleJavaScriptAlert(message: String, initiatedBy _: WebPage.FrameInfo) async {
        await withCheckedContinuation { continuation in
            self.present(message, confirm: false) { _ in continuation.resume() }
        }
    }

    func handleJavaScriptConfirm(
        message: String,
        initiatedBy _: WebPage.FrameInfo
    ) async -> WebPage.JavaScriptConfirmResult {
        await withCheckedContinuation { continuation in
            self.present(message, confirm: true) { continuation.resume(returning: $0 ? .ok : .cancel) }
        }
    }

    func handleJavaScriptPrompt(
        message _: String,
        defaultText _: String?,
        initiatedBy _: WebPage.FrameInfo
    ) async -> WebPage.JavaScriptPromptResult {
        .cancel
    }

    private func present(_ message: String, confirm: Bool, resume: @escaping (Bool) -> Void) {
        // One at a time: a second dialog while one is up answers the first
        // with "cancel" rather than leaving it hanging.
        self.answer(false)
        self.pending = Pending(message: message, isConfirm: confirm, resume: resume)
    }
}
