import Foundation

// MARK: Freshness: asking the file provider first

extension VaultSession {
    /// Before a refresh the reader asked for: has the vault's file provider
    /// update the folder on screen and the page's own file (see
    /// `ProviderRefresh`), within `providerBudget`, off the main thread.
    /// Nothing for a vault outside provider storage ("On My iPhone").
    func askProviderForUpdates() async {
        let root = self.app.rootPath()
        guard ProviderRefresh.isProviderLocation(root) else { return }
        let rootURL = URL(filePath: root, directoryHint: .isDirectory)
        let file = await self.shownSourceFile(root: rootURL)
        let listing = self.currentPath
        let singleFile = self.isSingleFile
        let budget = Self.providerBudget
        let (folder, outcome) = await Task.detached(priority: .userInitiated) {
            let folder = ProviderRefresh.folder(root: rootURL, file: file, listing: listing, singleFile: singleFile)
            return (folder, ProviderRefresh.refresh(folder: folder, file: file, budget: budget))
        }.value
        let shownFolder = folder.path(percentEncoded: false).dropFirst(root.count)
        Log.info("provider: refreshed \(shownFolder.isEmpty ? "/" : String(shownFolder)) "
            + "in \(outcome.millis) ms (\(ProviderRefresh.providerName(for: root))); "
            + "page file \(file == nil ? "none" : outcome.fileChanged ? "changed" : "unchanged"), "
            + "folder \(outcome.folderChanged ? "changed" : "unchanged"), "
            + "signal=\(outcome.signal ?? "none"), read=\(outcome.read)")
    }

    /// How long a refresh waits for the provider, at most.
    private static let providerBudget: Duration = .seconds(2)

    /// The markdown file behind the page on screen: the page's
    /// `markdown_source` (root-relative, `/`-separated), which only note pages
    /// carry. `nil` for a listing, or a source that would leave the root.
    private func shownSourceFile(root: URL) async -> URL? {
        guard self.page.url != nil,
              let source = try? await self.page.callJavaScript(
                  "return window.frontmatter?.markdown_source ?? null"
              ) as? String,
              !source.isEmpty,
              !source.split(separator: "/").contains("..")
        else { return nil }
        return root.appending(path: source, directoryHint: .notDirectory)
    }
}
