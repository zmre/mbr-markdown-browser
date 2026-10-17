import Foundation
import UIKit

/// Where Grant Folder Access opens the folder picker, and how to name that
/// folder to the reader in case the picker opens somewhere else.
///
/// `UIDocumentPickerViewController.directoryURL` is a hint ("Picker will try
/// to display this URL"): on a device (iOS 27.0.1, a Dropbox file) it was
/// ignored and the picker opened where the last pick was. What it is given
/// is therefore as close as possible to what the picker itself hands out:
/// the parent comes from the file's own `.parentDirectoryURLKey`, read while
/// the file's security scope is held (not `deletingLastPathComponent`, a
/// string edit that drops whatever the system attached to the URL), it keeps
/// the `/private/var` form the picker and Files use, and the file's scope
/// stays started for as long as the picker is up. Whatever the picker does
/// with it, `label` says in plain words which folder to pick.
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
            let provider = self.providerName(for: folder)
            let label = self.label(for: folder, provider: provider, device: device)
            Log.info("picker: start \(folder.path(percentEncoded: false)) "
                + "(parent from \(fromKey == nil ? "path" : "resource key"), file scope started=\(scoped), "
                + "provider=\(provider ?? "unknown"), label \"\(label)\")")
            return PickerStart(url: folder, label: label, fileAccess: access)
        }.value
    }

    // MARK: Naming

    /// The provider's own name for its root folder, when the system will
    /// say (`localizedName` of `…/CloudStorage/<id>` or of a third-party
    /// iCloud container); `nil` when all it has is an identifier.
    private static func providerName(for folder: URL) -> String? {
        let parts = folder.pathComponents
        let root: URL
        if let index = parts.firstIndex(of: "CloudStorage"), index + 1 < parts.count {
            root = URL(
                filePath: NSString.path(withComponents: Array(parts[...(index + 1)])),
                directoryHint: .isDirectory
            )
        } else if let index = parts.firstIndex(of: "Mobile Documents"), index + 1 < parts.count,
                  parts[index + 1] != "com~apple~CloudDocs" {
            root = URL(
                filePath: NSString.path(withComponents: Array(parts[...(index + 1)])),
                directoryHint: .isDirectory
            )
        } else {
            return nil
        }
        guard let name = (try? root.resourceValues(forKeys: [.localizedNameKey]))?.localizedName,
              !name.isEmpty, !self.looksLikeIdentifier(name), !name.contains("~")
        else { return nil }
        return name
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
