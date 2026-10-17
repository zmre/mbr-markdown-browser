import Foundation

/// Asks a file provider (Dropbox, iCloud Drive, OneDrive, …) to bring what the
/// reader is looking at up to date before a refresh stat-diffs the vault.
///
/// Why this is needed: a replicated File Provider keeps a local replica of
/// each folder it has materialized and updates it when *it* decides to. Plain
/// `readdir`/`stat` (all Rust's scan does) reads that replica and never tells
/// the provider anyone is interested, so a note edited on another machine can
/// stay stale on the phone indefinitely, while the Files app, which *observes*
/// the folder, sees the new version within a couple of seconds.
///
/// Which public API reaches the provider, from an app that does not own it:
/// - `NSFileProviderManager` is the provider's own API; another app cannot get
///   a manager for someone else's domain, and `NSMetadataQuery` only covers
///   the app's own ubiquity container.
/// - `URL.removeCachedResourceValue` / `removeAllCachedResourceValues` only
///   drop this process's cache of the replica's attributes; nothing reaches
///   the provider.
/// - A coordinated read with `.immediatelyAvailableMetadataOnly` is, by
///   definition, granted without the provider doing anything.
/// - **`NSFilePresenter`** is the documented lever. FileProvider's
///   `NSFileProviderEnumerating.enumerator(for:)` (NSFileProviderReplicated-
///   Extension.h) says the system keeps an enumerator open in the extension on
///   directories presented to the user and "on document on which an
///   application has a NSFilePresenter", and that the provider uses that open
///   enumerator as the hint to *prioritize delivery of updates* on the item or
///   its children. That is the mechanism the Files app's folder view triggers.
/// - A plain coordinated read (no options) of the page's file is added on top:
///   it waits for a pending download, so a file the provider evicted when its
///   remote version changed comes back with the new contents, and for an
///   older, non-replicated provider (`NSFileProviderExtension`) it is what
///   calls `startProvidingItem(at:)`, where those fetch the latest version.
///
/// So: register presenters on the folder and the page's file, make one
/// coordinated read of both (the folder's listing and the file's attributes
/// inside it), and wait — until a presenter is told something changed, the
/// file's or folder's stamp moves, or the budget runs out — then let the
/// ordinary stat-diff pick up whatever arrived. Everything runs off the main
/// thread; the coordinated read is cancelled at the deadline, so a provider
/// that never answers costs the budget and nothing more.
enum ProviderRefresh {
    struct Outcome {
        let millis: Int
        /// The page file's size or modification time differs afterwards.
        let fileChanged: Bool
        /// The folder's own modification time or entry count differs.
        let folderChanged: Bool
        /// What a presenter was told, if anything.
        let signal: String?
        /// The coordinated read finished within the budget (and how), or
        /// "timed out".
        let read: String
    }

    /// Whether `root` lives in a file provider's storage, i.e. whether asking
    /// the provider can change anything. A path test, so it costs nothing:
    /// `Library/CloudStorage` (replicated providers: Dropbox, OneDrive, Google
    /// Drive, …), `Library/Mobile Documents` (iCloud Drive) and an older
    /// provider's `File Provider Storage`. A folder "On My iPhone" is none of
    /// them: its files are the files.
    static func isProviderLocation(_ path: String) -> Bool {
        self.providerMarkers.contains { path.contains($0) }
    }

    private static let providerMarkers = [
        "/Library/CloudStorage/",
        "/Library/Mobile Documents/",
        "/File Provider Storage/"
    ]

    /// A name for the provider holding `path`, for logs. iOS gives third-party
    /// providers anonymous (UUID) `CloudStorage` folders, so those are just
    /// "file provider".
    static func providerName(for path: String) -> String {
        path.contains("/Library/Mobile Documents/") ? "iCloud" : "file provider"
    }

    /// The folder to refresh: the note's folder, else the listing on screen
    /// when it is a real folder (not a tag page, a static-folder overlay), else
    /// the root. A file opened in place has no folder access: just the root.
    static func folder(root: URL, file: URL?, listing: String?, singleFile: Bool) -> URL {
        if singleFile { return root }
        if let file { return file.deletingLastPathComponent() }
        if let listing, listing != "/", !listing.split(separator: "/").contains("..") {
            let candidate = root.appending(path: listing, directoryHint: .isDirectory)
            var isDirectory: ObjCBool = false
            if FileManager.default.fileExists(atPath: candidate.path(percentEncoded: false), isDirectory: &isDirectory),
               isDirectory.boolValue {
                return candidate
            }
        }
        return root
    }

    /// Asks the provider for fresh versions of `folder` and `file`, waiting at
    /// most `budget`. Never call it on the main thread: it blocks.
    static func refresh(folder: URL, file: URL?, budget: Duration) -> Outcome {
        let start = ContinuousClock.now
        let deadline = start + budget
        let fileBefore = file.flatMap(Self.stamp)
        let folderBefore = Self.folderStamp(folder)

        let signal = ChangeSignal()
        let presenters = [ItemPresenter(folder, signal: signal)]
            + (file.map { [ItemPresenter($0, signal: signal)] } ?? [])
        presenters.forEach(NSFileCoordinator.addFilePresenter)
        defer { presenters.forEach(NSFileCoordinator.removeFilePresenter) }

        // One coordinate call for both items, as NSFileCoordinator.h asks of
        // concurrent intents (several calls can deadlock across processes).
        let coordinator = NSFileCoordinator(filePresenter: nil)
        let intents = [NSFileAccessIntent.readingIntent(with: folder, options: [])]
            + (file.map { [NSFileAccessIntent.readingIntent(with: $0, options: [])] } ?? [])
        let queue = OperationQueue()
        queue.qualityOfService = .userInitiated
        coordinator.coordinate(with: intents, queue: queue) { error in
            if let error {
                signal.read("failed: \(error.localizedDescription)")
                return
            }
            // Inside the grant: list the folder and stat the file, which is
            // what a reader of either would do.
            let entries = (try? FileManager.default.contentsOfDirectory(atPath: intents[0].url.path(
                percentEncoded: false
            )))?.count ?? -1
            if intents.count > 1 {
                _ = Self.stamp(intents[1].url)
            }
            signal.read("\(entries) entries")
        }

        var fileChanged = false
        var folderChanged = false
        while ContinuousClock.now < deadline {
            fileChanged = file.flatMap(Self.stamp) != fileBefore
            folderChanged = Self.folderStamp(folder) != folderBefore
            if fileChanged || folderChanged || signal.changed != nil {
                // The provider is writing: give the write a moment to land.
                Thread.sleep(forTimeInterval: Self.settle)
                fileChanged = file.flatMap(Self.stamp) != fileBefore
                folderChanged = Self.folderStamp(folder) != folderBefore
                break
            }
            Thread.sleep(forTimeInterval: Self.pollInterval)
        }
        let read = signal.readResult ?? "timed out"
        if signal.readResult == nil {
            coordinator.cancel()
        }
        return Outcome(
            millis: millis(since: start),
            fileChanged: fileChanged,
            folderChanged: folderChanged,
            signal: signal.changed,
            read: read
        )
    }

    private static let pollInterval: TimeInterval = 0.15
    private static let settle: TimeInterval = 0.25

    /// Size and modification time, read fresh (`attributesOfItem` is a
    /// `stat`; a `URL`'s resource values would be cached per instance).
    private struct Stamp: Equatable {
        let size: UInt64
        let modified: Date?
        let entries: Int
    }

    private static func stamp(_ url: URL) -> Stamp? {
        guard let attributes = try? FileManager.default.attributesOfItem(atPath: url.path(percentEncoded: false))
        else { return nil }
        return Stamp(
            size: (attributes[.size] as? NSNumber)?.uint64Value ?? 0,
            modified: attributes[.modificationDate] as? Date,
            entries: 0
        )
    }

    /// The folder's own time plus how many names it holds: an added, removed
    /// or replaced (written-and-renamed) file moves one or the other. A file
    /// changed in place moves neither, which is what the presenter and the
    /// page file's own stamp are for.
    private static func folderStamp(_ url: URL) -> Stamp? {
        guard let own = self.stamp(url) else { return nil }
        let count = (try? FileManager.default.contentsOfDirectory(atPath: url.path(percentEncoded: false)))?.count
        return Stamp(size: 0, modified: own.modified, entries: count ?? -1)
    }
}

/// What the presenters and the coordinated read report, from their queues.
private final class ChangeSignal: @unchecked Sendable {
    private let lock = NSLock()
    private var changedValue: String?
    private var readValue: String?

    var changed: String? {
        self.lock.withLock { self.changedValue }
    }

    var readResult: String? {
        self.lock.withLock { self.readValue }
    }

    func change(_ what: String) {
        self.lock.withLock {
            if self.changedValue == nil { self.changedValue = what }
        }
    }

    func read(_ what: String) {
        self.lock.withLock { self.readValue = what }
    }
}

/// A presenter for one item, registered only while `ProviderRefresh` waits.
/// Its existence is the point (the provider opens an enumerator on a presented
/// item); the callbacks only end the wait early.
private final class ItemPresenter: NSObject, NSFilePresenter, @unchecked Sendable {
    let presentedItemURL: URL?
    let presentedItemOperationQueue: OperationQueue
    private let signal: ChangeSignal

    init(_ url: URL, signal: ChangeSignal) {
        self.presentedItemURL = url
        self.signal = signal
        let queue = OperationQueue()
        queue.maxConcurrentOperationCount = 1
        queue.qualityOfService = .utility
        self.presentedItemOperationQueue = queue
    }

    func presentedItemDidChange() {
        self.signal.change("\(self.presentedItemURL?.lastPathComponent ?? "?") changed")
    }

    func presentedSubitemDidChange(at url: URL) {
        self.signal.change("\(url.lastPathComponent) changed")
    }

    func presentedSubitemDidAppear(at url: URL) {
        self.signal.change("\(url.lastPathComponent) appeared")
    }

    func presentedItemDidGain(_: NSFileVersion) {
        self.signal.change("\(self.presentedItemURL?.lastPathComponent ?? "?") gained a version")
    }
}
