import Foundation
import Observation
import WebKit

// The explicit, formally redundant raw values below are the literals
// `app-bridge.test.ts` looks for; swiftformat would otherwise strip them.
// (swiftlint's redundant_string_enum_value fires only when every value is
// redundant, and `newNote = "new"` / `linksOut = "links-out"` are not, so it
// needs no fence.)
// swiftformat:disable redundantRawValues

/// An action the page can perform, as `components/src/app-bridge.ts` names
/// it. The raw values are the bridge's wire names; `app-bridge.test.ts` reads
/// this file and checks every one of them, so a rename on either side fails a
/// test rather than silently emptying the menu.
enum AppAction: String, CaseIterable, Identifiable {
    case browse = "browse"
    case search = "search"
    case edit = "edit"
    case info = "info"
    case toc = "toc"
    case linksOut = "links-out"
    case linksIn = "links-in"
    case tasks = "tasks"
    case flashcards = "flashcards"
    case slides = "slides"
    case newNote = "new"
    // swiftformat:enable redundantRawValues

    var id: String {
        rawValue
    }

    var label: String {
        switch self {
        case .browse: "Browse"
        case .search: "Search"
        case .edit: "Edit"
        case .info: "Info"
        case .toc: "Contents"
        case .linksOut: "Links Out"
        case .linksIn: "Links In"
        case .tasks: "Tasks"
        case .flashcards: "Flashcards"
        case .slides: "Slides"
        case .newNote: "New Note"
        }
    }

    var symbol: String {
        switch self {
        case .browse: "sidebar.left"
        case .search: "magnifyingglass"
        case .edit: "pencil"
        case .info: "info.circle"
        case .toc: "list.bullet.indent"
        case .linksOut: "arrow.up.right"
        case .linksIn: "arrow.down.left"
        case .tasks: "checklist"
        case .flashcards: "rectangle.on.rectangle.angled"
        case .slides: "play.rectangle"
        case .newNote: "plus"
        }
    }
}

/// One step of the breadcrumb path.
struct Crumb: Identifiable, Hashable {
    let name: String
    /// Site-absolute URL path (`/notes/`).
    let url: String
    var id: String {
        self.url
    }
}

/// What the page on screen reported about itself (`postPageState`, and
/// `postEditorState` while the full-screen editor is up).
@MainActor
@Observable
final class PageInfo {
    private(set) var title = ""
    private(set) var breadcrumbs: [Crumb] = []
    private(set) var actions: [AppAction] = []
    /// The page's full-screen editor is open: the app hides its bar and turns
    /// off the edge swipes, so neither can throw away unsaved edits.
    /// Every page message sets it again (a page load ends any editor; the
    /// page's `load` re-post carries an editor that opened with the page).
    private(set) var editorOpen = false
    /// A slide presentation is playing: the app hides its bar (a landscape
    /// deck is squashed under it) and turns off the edge swipes, which
    /// Reveal's own slide swipes would trip. Reset by every page message, like
    /// `editorOpen`; leaving a deck (✕) reloads the page.
    private(set) var slidesPlaying = false

    /// Applies one `{type: 'page', ...}`, `{type: 'editor', open}` or
    /// `{type: 'slides', playing}` message.
    /// Anything malformed is ignored: the bar keeps what it had.
    func apply(_ body: Any) {
        guard let message = body as? [String: Any] else { return }
        switch message["type"] as? String {
        case "page": self.applyPage(message)
        case "editor":
            if let open = message["open"] as? Bool { self.editorOpen = open }
        case "slides":
            if let playing = message["playing"] as? Bool { self.slidesPlaying = playing }
        default: break
        }
    }

    private func applyPage(_ message: [String: Any]) {
        self.editorOpen = (message["editorOpen"] as? Bool) ?? false
        self.slidesPlaying = (message["slidesPlaying"] as? Bool) ?? false
        self.title = (message["title"] as? String) ?? ""
        self.breadcrumbs = ((message["breadcrumbs"] as? [[String: Any]]) ?? []).compactMap { crumb in
            guard let name = crumb["name"] as? String, let url = crumb["url"] as? String else { return nil }
            return Crumb(name: name, url: url)
        }
        self.actions = ((message["actions"] as? [String]) ?? []).compactMap(AppAction.init(rawValue:))
    }
}

/// Wires a `WebPage.Configuration` for the app shell: marks every page as
/// embedded before any of its scripts run, and receives the page's reports.
enum AppBridge {
    /// The script-message handler name `app-bridge.ts` posts to.
    static let messageHandler = "mbrApp"

    /// Runs at document start in the main frame: `app-bridge.ts`'s
    /// `isEmbedded()` reads the global, and theme.css hides mbr's own header
    /// and breadcrumbs under the class.
    /// `offline` adds `mbr-offline`, which only the Downloading page reads
    /// (see `NetworkStatus`).
    static func embeddedScript(offline: Bool) -> String {
        """
        window.__MBR_EMBEDDED__ = true;
        document.documentElement.classList.add('mbr-embedded');
        document.documentElement.classList.toggle('mbr-offline', \(offline));
        """
    }

    @MainActor
    static func install(in configuration: inout WebPage.Configuration, info: PageInfo, offline: Bool) {
        let controller = configuration.userContentController
        self.addDocumentStartScript(to: controller, offline: offline)
        controller.add(MessageReceiver(info: info), name: self.messageHandler)
    }

    /// Network state changed: later documents start with the new class (the
    /// script is replaced; message handlers are untouched), and the page on
    /// screen switches now.
    @MainActor
    static func setOffline(_ offline: Bool, controller: WKUserContentController, page: WebPage) async {
        controller.removeAllUserScripts()
        self.addDocumentStartScript(to: controller, offline: offline)
        do {
            _ = try await page.callJavaScript(
                "document.documentElement.classList.toggle('mbr-offline', offline)",
                arguments: ["offline": offline]
            )
        } catch {
            Log.info("bridge: offline=\(offline) not applied to the page: \(error)")
        }
    }

    @MainActor
    private static func addDocumentStartScript(to controller: WKUserContentController, offline: Bool) {
        controller.addUserScript(WKUserScript(
            source: self.embeddedScript(offline: offline), injectionTime: .atDocumentStart, forMainFrameOnly: true
        ))
    }

    /// Asks the page to open `action` through `window.mbrApp.open`, the same
    /// public method its own button uses. `false` when the page declined (not
    /// available there) or the call failed.
    @MainActor
    static func open(_ action: AppAction, in page: WebPage) async -> Bool {
        do {
            let opened = try await page.callJavaScript(
                "return window.mbrApp ? window.mbrApp.open(name) : false",
                arguments: ["name": action.rawValue]
            )
            return (opened as? Bool) ?? false
        } catch {
            Log.info("bridge: open \(action.rawValue) failed: \(error)")
            return false
        }
    }

    /// Receives the page's messages. WebKit holds it strongly; it holds the
    /// model weakly, so a closed page leaves nothing behind.
    private final class MessageReceiver: NSObject, WKScriptMessageHandler {
        private weak var info: PageInfo?

        init(info: PageInfo) {
            self.info = info
        }

        func userContentController(_: WKUserContentController, didReceive message: WKScriptMessage) {
            let body = message.body
            MainActor.assumeIsolated {
                self.info?.apply(body)
            }
        }
    }
}
