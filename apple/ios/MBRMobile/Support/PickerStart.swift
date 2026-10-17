import Foundation
import UIKit

/// Where Grant Folder Access opens the folder picker, and how to name that
/// folder to the reader in case the picker opens somewhere else.
///
/// The start is the file's own `.parentDirectoryURLKey`, read with the
/// file's security scope started, and the scope stays started while the
/// picker is up. That is harmless and is the best `directoryURL` the app
/// can give, but iOS 27.0.1 still ignored it for a Dropbox folder (device,
/// with the scope started and the parent from the resource key): the
/// picker opened at the last folder picked. For a third-party File
/// Provider folder, `label` is how the reader finds it.
final class PickerStart: @unchecked Sendable {
    /// The folder to open in.
    let url: URL
    /// `Dropbox › Dev › elovation`.
    let label: String
    /// The file's security scope, held while the picker is up.
    private let fileAccess: FolderAccess

    private init(url: URL, label: String, fileAccess: FolderAccess) {
        self.url = url
        self.label = label
        self.fileAccess = fileAccess
    }

    /// The folder around `file`, with the file's scope started.
    static func resolve(file: URL) async -> PickerStart {
        let device = await UIDevice.current.model
        return await Task.detached(priority: .userInitiated) {
            let scoped = file.startAccessingSecurityScopedResource()
            let access = FolderAccess(url: file, scoped: scoped)
            let fromKey = (try? file.resourceValues(forKeys: [.parentDirectoryURLKey]))?.parentDirectory
            let parent = (fromKey ?? file.deletingLastPathComponent()).standardizedFileURL
            let folder = URL(filePath: parent.path(percentEncoded: false), directoryHint: .isDirectory)
            let provider = self.providerName(for: folder, file: file)
            let label = self.label(for: folder, provider: provider, device: device)
            Log.info("picker: start \(folder.path(percentEncoded: false)) "
                + "(parent from \(fromKey == nil ? "path" : "resource key"), file scope started=\(scoped), "
                + "provider=\(provider ?? "unknown"), label \"\(label)\")")
            return PickerStart(url: folder, label: label, fileAccess: access)
        }.value
    }

    // MARK: Naming

    /// The provider's own name for its root folder, when the system will
    /// say; `nil` when all it has is an identifier (iOS names a provider's
    /// `CloudStorage` folder by a UUID). Asked three ways, each logged, run
    /// with the file's scope started: `localizedName` and
    /// `FileManager.displayName` of the root (`…/CloudStorage/<id>` or a
    /// third-party iCloud container), then the file's
    /// `ubiquitousItemContainerDisplayName`. Without an entitlement the
    /// first two may only echo the folder's own name.
    /// (`NSFileProviderManager.getIdentifierForUserVisibleFile` gives a
    /// domain identifier, not a name, and listing domains is the
    /// provider's own business.)
    private static func providerName(for folder: URL, file: URL) -> String? {
        var tried: [String] = []
        defer { Log.info("picker: provider name \(tried.joined(separator: ", "))") }
        func usable(_ name: String?, _ how: String) -> String? {
            tried.append("\(how)=\(name.map { "\"\($0)\"" } ?? "nil")")
            guard let name, !name.isEmpty, !self.looksLikeIdentifier(name), !name.contains("~") else { return nil }
            return name
        }
        if let root = self.providerRoot(of: folder) {
            let localized = (try? root.resourceValues(forKeys: [.localizedNameKey]))?.localizedName
            if let name = usable(localized, "localizedName") { return name }
            let display = FileManager.default.displayName(atPath: root.path(percentEncoded: false))
            if let name = usable(display, "displayName") { return name }
        }
        let container = (try? file.resourceValues(forKeys: [.ubiquitousItemContainerDisplayNameKey]))?
            .ubiquitousItemContainerDisplayName
        return usable(container, "containerDisplayName")
    }

    /// `…/CloudStorage/<id>`, or a third-party iCloud container; `nil`
    /// elsewhere.
    private static func providerRoot(of folder: URL) -> URL? {
        let parts = folder.pathComponents
        let end: Int
        if let index = parts.firstIndex(of: "CloudStorage"), index + 1 < parts.count {
            end = index + 1
        } else if let index = parts.firstIndex(of: "Mobile Documents"), index + 1 < parts.count,
                  parts[index + 1] != "com~apple~CloudDocs" {
            end = index + 1
        } else {
            return nil
        }
        return URL(filePath: NSString.path(withComponents: Array(parts[...end])), directoryHint: .isDirectory)
    }

    /// The folder as a reader would find it in the picker:
    /// `Dropbox › Dev › elovation`, `iCloud Drive › Notes`,
    /// `On My iPhone › MBR › Test`. Without a provider name, just the folders
    /// below the provider's root. Pure, for the self-test.
    static func label(for folder: URL, provider: String?, device: String = "iPhone") -> String {
        let parts = folder.standardizedFileURL.pathComponents
        var head: [String] = []
        var rest: ArraySlice<String>
        if let index = parts.firstIndex(of: "CloudStorage"), index + 1 < parts.count {
            let root = parts[index + 1]
            if let provider {
                head = [provider]
            } else if !self.looksLikeIdentifier(root) {
                // macOS-style `GoogleDrive-name@example.com`, `Dropbox`.
                head = [String(root.split(separator: "-", maxSplits: 1).first ?? Substring(root))]
            }
            rest = parts[(index + 2)...]
        } else if let index = parts.firstIndex(of: "Mobile Documents"), index + 1 < parts.count {
            let container = parts[index + 1]
            head = ["iCloud Drive"]
            rest = parts[(index + 2)...]
            if container != "com~apple~CloudDocs" {
                // An app's folder: `iCloud~com~example~App/Documents/…`.
                let app = provider ?? String(container.split(separator: "~").last ?? Substring(container))
                head.append(app)
                if rest.first == "Documents" { rest = rest.dropFirst() }
            }
        } else if let index = parts.firstIndex(of: "File Provider Storage") {
            head = provider.map { [$0] } ?? []
            rest = parts[(index + 1)...]
        } else if let index = parts.firstIndex(of: "Documents"), index >= 2,
                  parts[index - 2] == "Application" {
            // An app's own Documents: "On My iPhone › <app>".
            let app = parts[index - 1] == self.ownContainer ? "MBR" : (provider ?? "")
            head = ["On My \(device)"] + (app.isEmpty ? [] : [app])
            rest = parts[(index + 1)...]
        } else {
            head = provider.map { [$0] } ?? []
            rest = parts.dropFirst().suffix(3)
        }
        let all = head + rest
        return all.isEmpty ? folder.lastPathComponent : all.joined(separator: " › ")
    }

    /// The UUID of this app's data container (`…/Application/<uuid>/Documents`).
    private static let ownContainer: String = AppPaths.documents.deletingLastPathComponent().lastPathComponent

    /// A UUID rather than a name a person would know (iOS names a
    /// provider's `CloudStorage` folder by one).
    private static func looksLikeIdentifier(_ name: String) -> Bool {
        UUID(uuidString: name) != nil
    }
}
