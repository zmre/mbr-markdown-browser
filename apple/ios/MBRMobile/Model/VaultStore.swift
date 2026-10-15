import Foundation
import Observation

/// A folder the user added, or one inside the app's own container (the
/// bundled sample, and folders the self-test creates).
struct Vault: Identifiable, Codable, Hashable {
    enum Location: Codable, Hashable {
        /// Plain `bookmarkData()` of a picked folder. (`.withSecurityScope`
        /// is macOS-only; on iOS a plain bookmark of a security-scoped URL
        /// carries the grant.)
        case bookmark(Data)
        /// A folder under the app's Documents, by relative path. Ours, so no
        /// grant is needed.
        case container(String)
    }

    var id: UUID
    /// The display name. Starts as the folder's name; the user can rename it.
    var name: String
    var location: Location

    var isContainer: Bool {
        if case .container = self.location { return true }
        return false
    }
}

/// Where the app keeps things.
enum AppPaths {
    /// The bundled sample vault, copied into Documents so it is writable and
    /// visible in Files.
    static let sampleFolder = "Sample Vault"

    static var documents: URL {
        URL.documentsDirectory
    }

    /// Every vault's index snapshot lives in its own folder under here, named
    /// by the vault's id, so removing a vault or clearing the cache is one
    /// directory delete.
    static var vaultCaches: URL {
        URL.cachesDirectory.appending(path: "vaults", directoryHint: .isDirectory)
    }

    static func cacheFolder(for id: UUID) -> URL {
        self.vaultCaches.appending(path: id.uuidString, directoryHint: .isDirectory)
    }
}

/// Security-scoped access to a vault's folder (or a single file), held for as
/// long as the `MbrApp` reading it lives and released when this is.
final class FolderAccess: @unchecked Sendable {
    let url: URL
    private let scoped: Bool

    init(url: URL, scoped: Bool) {
        self.url = url
        self.scoped = scoped
    }

    deinit {
        if scoped { url.stopAccessingSecurityScopedResource() }
    }

    var path: String {
        self.url.path(percentEncoded: false)
    }
}

enum VaultError: LocalizedError {
    case accessDenied(String)
    case bookmarkUnresolvable(String)

    var errorDescription: String? {
        switch self {
        case let .accessDenied(path):
            "iOS no longer lets MBR read \(path). Pick the folder again to restore access."
        case let .bookmarkUnresolvable(reason):
            "MBR could not find this folder any more (\(reason)). Pick it again to restore access."
        }
    }
}

/// The vault list, persisted in `UserDefaults`.
@MainActor
@Observable
final class VaultStore {
    private(set) var vaults: [Vault] = []
    var lastError: String?

    /// Not the spike's key: its records had another shape.
    private let defaultsKey = "mbr.vaults.v1"

    init() {
        self.load()
        if !self.vaults.contains(where: { $0.location == .container(AppPaths.sampleFolder) }) {
            self.vaults.insert(
                Vault(id: UUID(), name: "Sample Vault (mbr docs)", location: .container(AppPaths.sampleFolder)),
                at: 0
            )
            self.save()
        }
    }

    func vault(_ id: UUID) -> Vault? {
        self.vaults.first { $0.id == id }
    }

    // MARK: Changes

    /// Remembers a folder the picker returned. The bookmark is made with the
    /// picker's security scope held, off the main thread (it can touch the
    /// file provider).
    @discardableResult
    func add(folder url: URL) async -> Vault? {
        do {
            let bookmark = try await Self.makeBookmark(url)
            let vault = Vault(id: UUID(), name: url.lastPathComponent, location: .bookmark(bookmark))
            self.vaults.append(vault)
            self.save()
            Log.info("vault added: \(vault.name) at \(url.path(percentEncoded: false))")
            return vault
        } catch {
            self.lastError = "Could not remember that folder: \(error.localizedDescription)"
            return nil
        }
    }

    /// A vault inside the app's container (the self-test's), added once.
    @discardableResult
    func addContainerFolder(_ relativePath: String, name: String) -> Vault {
        if let existing = vaults.first(where: { $0.location == .container(relativePath) }) {
            return existing
        }
        let vault = Vault(id: UUID(), name: name, location: .container(relativePath))
        self.vaults.append(vault)
        self.save()
        return vault
    }

    /// Points an existing vault at a newly picked folder (the recovery for a
    /// bookmark that stopped resolving). Keeps its id, so its name and index
    /// cache carry over.
    func repick(_ id: UUID, folder url: URL) async -> Bool {
        do {
            let bookmark = try await Self.makeBookmark(url)
            guard let index = vaults.firstIndex(where: { $0.id == id }) else { return false }
            self.vaults[index].location = .bookmark(bookmark)
            self.save()
            Log.info("vault re-picked: \(self.vaults[index].name) at \(url.path(percentEncoded: false))")
            return true
        } catch {
            self.lastError = "Could not remember that folder: \(error.localizedDescription)"
            return false
        }
    }

    func rename(_ id: UUID, to name: String) {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, let index = vaults.firstIndex(where: { $0.id == id }) else { return }
        self.vaults[index].name = trimmed
        self.save()
    }

    /// Forgets a vault and deletes its index cache. The folder itself is
    /// never touched.
    func remove(_ id: UUID) {
        self.vaults.removeAll { $0.id == id }
        self.save()
        let cache = AppPaths.cacheFolder(for: id)
        Task.detached(priority: .utility) {
            try? FileManager.default.removeItem(at: cache)
        }
    }

    // MARK: Opening

    /// Resolves a vault's folder and starts security-scoped access to it.
    /// Runs off the main thread: resolving a bookmark can wait on the file
    /// provider. A stale bookmark is re-saved.
    func access(_ vault: Vault) async throws -> FolderAccess {
        switch vault.location {
        case let .container(relativePath):
            let url = AppPaths.documents.appending(path: relativePath, directoryHint: .isDirectory)
            if relativePath == AppPaths.sampleFolder {
                try await Self.installSample(at: url)
            }
            return FolderAccess(url: url, scoped: false)
        case let .bookmark(bookmark):
            let (access, fresh) = try await Task.detached(priority: .userInitiated) {
                try Self.resolve(bookmark)
            }.value
            if let fresh, let index = vaults.firstIndex(where: { $0.id == vault.id }) {
                Log.info("bookmark stale for \(vault.name); refreshed")
                self.vaults[index].location = .bookmark(fresh)
                self.save()
            }
            return access
        }
    }

    /// Resolves `bookmark`, starts access, and returns a fresh bookmark when
    /// the old one was stale.
    nonisolated static func resolve(_ bookmark: Data) throws -> (FolderAccess, Data?) {
        var isStale = false
        let url: URL
        do {
            url = try URL(resolvingBookmarkData: bookmark, options: [], relativeTo: nil, bookmarkDataIsStale: &isStale)
        } catch {
            throw VaultError.bookmarkUnresolvable(error.localizedDescription)
        }
        let scoped = url.startAccessingSecurityScopedResource()
        let access = FolderAccess(url: url, scoped: scoped)
        guard FileManager.default.isReadableFile(atPath: access.path) else {
            throw VaultError.accessDenied(access.path)
        }
        let fresh = isStale ? try? url.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil)
            : nil
        return (access, fresh)
    }

    nonisolated static func makeBookmark(_ url: URL) async throws -> Data {
        try await Task.detached(priority: .userInitiated) {
            let scoped = url.startAccessingSecurityScopedResource()
            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
            return try url.bookmarkData(options: [], includingResourceValuesForKeys: nil, relativeTo: nil)
        }.value
    }

    /// Copies the bundled docs into Documents the first time the sample is
    /// opened.
    nonisolated static func installSample(at url: URL) async throws {
        try await Task.detached(priority: .userInitiated) {
            let fileManager = FileManager.default
            guard !fileManager.fileExists(atPath: url.path(percentEncoded: false)),
                  let bundled = Bundle.main.url(forResource: "docs", withExtension: nil)
            else { return }
            try fileManager.copyItem(at: bundled, to: url)
            Log.info("sample vault installed at \(url.path(percentEncoded: false))")
        }.value
    }

    // MARK: Persistence

    private func load() {
        guard let data = UserDefaults.standard.data(forKey: defaultsKey) else { return }
        do {
            self.vaults = try JSONDecoder().decode([Vault].self, from: data)
        } catch {
            Log.info("vault list unreadable, starting empty: \(error)")
        }
    }

    private func save() {
        do {
            try UserDefaults.standard.set(JSONEncoder().encode(self.vaults), forKey: self.defaultsKey)
        } catch {
            Log.info("vault list not saved: \(error)")
        }
    }
}
