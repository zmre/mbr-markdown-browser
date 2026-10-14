import Foundation

/// mbr's `FileCoordinator`: every write, move, delete and mkdir the Rust core
/// makes in a vault goes through `NSFileCoordinator`, so the file provider
/// (iCloud Drive, Dropbox, Working Copy, …) sees a coordinated change. Reads
/// never come here; Rust reads with plain `std::fs`.
///
/// Rust calls these from its own background threads and waits for them, so
/// each method blocks until the operation is done, which is what a coordinated
/// write does anyway. None of them touches the main thread.
final class FileCoordinatorBridge: FileCoordinator, @unchecked Sendable {
    private let lock = NSLock()
    private var counts: [String: Int] = [:]

    /// How many times `operation` (`write`, `mkdir`, `move`, `remove`)
    /// succeeded, for the self-test.
    func count(of operation: String) -> Int {
        self.lock.withLock { self.counts[operation, default: 0] }
    }

    private func record(_ operation: String, _ path: String, since start: ContinuousClock.Instant, _ how: String) {
        self.lock.withLock { self.counts[operation, default: 0] += 1 }
        Log.info("coordinator: \(operation) \(path) (\(how)) in \(millis(since: start))ms")
    }

    // MARK: FileCoordinator

    /// A `.forReplacing` coordinated write: the new contents go to a file in
    /// the item-replacement directory for the target's volume, then
    /// `replaceItemAt` swaps it in. Never a move out of
    /// `NSTemporaryDirectory()`, which may be another volume. Where the
    /// provider cannot do that (SMB shares refuse atomic replacement), the
    /// fallback is a plain coordinated overwrite.
    func coordinatedWrite(path: String, contents: Data) throws {
        let start = ContinuousClock.now
        var coordinationError: NSError?
        var failure: Error?
        var how = "replace"
        NSFileCoordinator(filePresenter: nil).coordinate(
            writingItemAt: URL(filePath: path), options: .forReplacing, error: &coordinationError
        ) { target in
            do {
                how = try Self.replace(target, with: contents)
            } catch {
                Log.info("coordinator: atomic replace of \(path) failed (\(error.localizedDescription)); "
                    + "writing in place")
                how = "in place"
                do { try contents.write(to: target) } catch { failure = error }
            }
        }
        try Self.check(coordinationError ?? failure, path: path)
        self.record("write", path, since: start, how)
    }

    func coordinatedCreateDirectory(path: String) throws {
        let start = ContinuousClock.now
        var coordinationError: NSError?
        var failure: Error?
        NSFileCoordinator(filePresenter: nil).coordinate(
            writingItemAt: URL(filePath: path, directoryHint: .isDirectory), options: [], error: &coordinationError
        ) { target in
            do {
                try FileManager.default.createDirectory(at: target, withIntermediateDirectories: true)
            } catch { failure = error }
        }
        try Self.check(coordinationError ?? failure, path: path)
        self.record("mkdir", path, since: start, "create")
    }

    func coordinatedMove(from: String, to: String) throws {
        let start = ContinuousClock.now
        let coordinator = NSFileCoordinator(filePresenter: nil)
        var coordinationError: NSError?
        var failure: Error?
        coordinator.coordinate(
            writingItemAt: URL(filePath: from),
            options: .forMoving,
            writingItemAt: URL(filePath: to),
            options: .forReplacing,
            error: &coordinationError
        ) { source, destination in
            do {
                let fileManager = FileManager.default
                if fileManager.fileExists(atPath: destination.path(percentEncoded: false)) {
                    _ = try fileManager.replaceItemAt(destination, withItemAt: source)
                } else {
                    try fileManager.moveItem(at: source, to: destination)
                }
                coordinator.item(at: source, didMoveTo: destination)
            } catch { failure = error }
        }
        try Self.check(coordinationError ?? failure, path: from)
        self.record("move", "\(from) -> \(to)", since: start, "move")
    }

    func coordinatedRemove(path: String) throws {
        let start = ContinuousClock.now
        var coordinationError: NSError?
        var failure: Error?
        NSFileCoordinator(filePresenter: nil).coordinate(
            writingItemAt: URL(filePath: path), options: .forDeleting, error: &coordinationError
        ) { target in
            do { try FileManager.default.removeItem(at: target) } catch { failure = error }
        }
        try Self.check(coordinationError ?? failure, path: path)
        self.record("remove", path, since: start, "delete")
    }

    /// Returns at once; the downloads happen on a background queue. iCloud
    /// items are asked to download; anything else gets a coordinated read,
    /// which is what makes a third-party provider (Dropbox, Working Copy)
    /// materialize the file.
    func requestDownload(paths: [String]) {
        Log.info("coordinator: download requested for \(paths.count) file(s): "
            + paths.prefix(3).joined(separator: ", "))
        DispatchQueue.global(qos: .utility).async {
            for path in paths {
                Self.download(URL(filePath: path))
            }
        }
    }

    // MARK: Helpers

    /// Throws the failure of a coordinated operation, if there was one, as a
    /// `CoordinatorError`.
    private static func check(_ error: Error?, path: String) throws {
        guard let error else { return }
        Log.info("coordinator: \(path) failed: \(error.localizedDescription)")
        throw self.coordinatorError(error, path: path)
    }

    /// `replaceItemAt` from a sibling in the item-replacement directory.
    /// Creates the file directly when there is nothing to replace yet.
    private static func replace(_ target: URL, with contents: Data) throws -> String {
        let fileManager = FileManager.default
        guard fileManager.fileExists(atPath: target.path(percentEncoded: false)) else {
            try contents.write(to: target, options: .withoutOverwriting)
            return "create"
        }
        let scratch = try fileManager.url(
            for: .itemReplacementDirectory, in: .userDomainMask, appropriateFor: target, create: true
        )
        defer { try? fileManager.removeItem(at: scratch) }
        let staged = scratch.appending(path: target.lastPathComponent)
        try contents.write(to: staged)
        _ = try fileManager.replaceItemAt(target, withItemAt: staged)
        return "replace"
    }

    private static func download(_ url: URL) {
        let start = ContinuousClock.now
        let path = url.path(percentEncoded: false)
        let ubiquitous = (try? url.resourceValues(forKeys: [.isUbiquitousItemKey]))?.isUbiquitousItem ?? false
        if ubiquitous {
            do {
                try FileManager.default.startDownloadingUbiquitousItem(at: url)
                Log.info("coordinator: iCloud download started for \(path)")
            } catch {
                Log.info("coordinator: iCloud download of \(path) failed: \(error.localizedDescription)")
            }
            return
        }
        var coordinationError: NSError?
        var bytes = -1
        NSFileCoordinator(filePresenter: nil).coordinate(
            readingItemAt: url, options: [], error: &coordinationError
        ) { target in
            bytes = (try? Data(contentsOf: target, options: .mappedIfSafe))?.count ?? -1
        }
        if let coordinationError {
            Log.info("coordinator: coordinated read of \(path) failed: \(coordinationError.localizedDescription)")
        } else {
            Log.info("coordinator: coordinated read of \(path) (\(bytes)B) in \(millis(since: start))ms")
        }
    }

    static func coordinatorError(_ error: Error, path: String) -> CoordinatorError {
        if let error = error as? CoordinatorError { return error }
        let nsError = error as NSError
        let message = "\(path): \(nsError.localizedDescription)"
        let underlying = nsError.userInfo[NSUnderlyingErrorKey] as? NSError
        for candidate in [nsError, underlying].compactMap(\.self) {
            switch (candidate.domain, candidate.code) {
            case (NSCocoaErrorDomain, NSFileNoSuchFileError),
                 (NSCocoaErrorDomain, NSFileReadNoSuchFileError),
                 (NSPOSIXErrorDomain, Int(ENOENT)):
                return .NotFound(message: message)
            case (NSCocoaErrorDomain, NSFileWriteNoPermissionError),
                 (NSCocoaErrorDomain, NSFileReadNoPermissionError),
                 (NSPOSIXErrorDomain, Int(EACCES)),
                 (NSPOSIXErrorDomain, Int(EPERM)):
                return .PermissionDenied(message: message)
            default:
                continue
            }
        }
        return .Failed(message: message)
    }
}
