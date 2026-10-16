import SwiftUI
import WebKit

/// The navigation bar's Back: web-history back, and out to the vault list at
/// the start of the vault's history. A long press lists the vault's earlier
/// pages, as Safari's does.
struct HistoryBackButton: View {
    let session: VaultSession
    let leave: () -> Void

    var body: some View {
        Menu {
            ForEach(self.session.recentPages.prefix(15)) { item in
                Button(Self.label(for: item)) {
                    Task { await self.session.go(to: item) }
                }
            }
            if !self.session.recentPages.isEmpty {
                Divider()
            }
            Button("Vaults", systemImage: "books.vertical", action: self.leave)
        } label: {
            Label("Back", systemImage: "chevron.backward")
        } primaryAction: {
            if self.session.canGoBack {
                Task { await self.session.goBack() }
            } else {
                self.leave()
            }
        }
        .accessibilityHint(self.session.canGoBack ? "Goes to the previous page" : "Returns to the vault list")
    }

    private static func label(for item: WebPage.BackForwardList.Item) -> String {
        if let title = item.title?.trimmingCharacters(in: .whitespacesAndNewlines), !title.isEmpty {
            return title
        }
        return item.url.path(percentEncoded: false)
    }
}

/// The ⋯ menu: the page's own actions (what `app-bridge.ts` reported it
/// supports), then the app's. New Note is not here: it is the bar's own `+`.
/// Search is the native `SearchSheet`, not the page's modal: WebKit would
/// focus the modal's input but keep the keyboard down for a menu tap.
struct VaultActionsMenu: View {
    let session: VaultSession
    let openSearch: () -> Void
    let openSettings: () -> Void
    let leave: () -> Void

    var body: some View {
        Menu("More", systemImage: "ellipsis") {
            Section {
                ForEach(self.session.web.info.actions.filter { $0 != .newNote }) { action in
                    Button(action.label, systemImage: action.symbol) {
                        Task { await self.perform(action) }
                    }
                }
            }
            Section {
                Button("Refresh", systemImage: "arrow.clockwise") {
                    Task { await self.session.refresh(reason: "button") }
                }
                Button("Home", systemImage: "house") {
                    Task {
                        await self.session.load(
                            MbrSchemeHandler.url(forPath: self.session.app.homeUrl()), label: "home"
                        )
                    }
                }
            }
            Section {
                Button("Settings", systemImage: "gear", action: self.openSettings)
                Button("Open Other Vault…", systemImage: "books.vertical", action: self.leave)
            }
        }
    }

    private func perform(_ action: AppAction) async {
        if action == .search {
            self.openSearch()
            return
        }
        let opened = await AppBridge.open(action, in: self.session.page)
        Log.info("bridge: open \(action.rawValue) -> \(opened)")
    }
}

/// The title's menu: the breadcrumb path above the page.
struct BreadcrumbMenu: View {
    let session: VaultSession

    var body: some View {
        ForEach(self.session.web.info.breadcrumbs.reversed()) { crumb in
            Button(crumb.name, systemImage: crumb.url == "/" ? "house" : "folder") {
                Task { await self.session.load(MbrSchemeHandler.url(forPath: crumb.url), label: "breadcrumb") }
            }
        }
    }
}
