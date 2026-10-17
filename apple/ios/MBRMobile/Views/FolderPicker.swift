import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// The system folder picker, opening in `start` when given.
///
/// `UIDocumentPickerViewController` rather than `.fileImporter`: on a device,
/// `.fileDialogDefaultDirectory` was ignored for a file opened from a
/// provider (Grant Folder Access started somewhere else), while the picker's
/// own `directoryURL` is the documented way to choose where it opens.
///
/// Presented by UIKit from the top view controller, not wrapped in a SwiftUI
/// `.sheet`: as a representable inside a sheet, the picker opened at the
/// root of On My iPhone with `directoryURL` set (simulator, iOS 27), and
/// drew itself over anything placed beside it in the sheet.
///
/// `directoryURL` is only a hint ("Picker will try to display this URL"),
/// and iOS 27.0.1 ignores it for a third-party File Provider folder
/// (observed with Dropbox: the picker opened at the last folder picked).
/// So Grant Folder Access names the folder in words (`FolderPickerHint`)
/// before the picker opens. Always at full height: any shorter detent
/// (which would have kept the hint visible above it) draws the picker's
/// sheet as light glass in dark mode, under its white text.
@MainActor
enum FolderPicker {
    /// The delegate of the picker on screen; UIKit holds delegates weakly.
    private static var current: Coordinator?

    /// Presents the picker. `done` gets the folder picked, or `nil` when it
    /// was cancelled (or could not be presented).
    static func present(start: URL?, done: @escaping (URL?) -> Void) {
        guard let host = self.topViewController() else {
            Log.info("picker: no view controller to present from")
            done(nil)
            return
        }
        let picker = UIDocumentPickerViewController(forOpeningContentTypes: [.folder], asCopy: false)
        picker.allowsMultipleSelection = false
        picker.directoryURL = start
        let coordinator = Coordinator { folder in
            self.current = nil
            done(folder)
        }
        self.current = coordinator
        picker.delegate = coordinator
        let place = start?.path(percentEncoded: false) ?? "the last place"
        let set = picker.directoryURL?.path(percentEncoded: false) ?? "nil"
        Log.info("picker: opening in \(place) (directoryURL now \(set))")
        host.present(picker, animated: true)
    }

    private static func topViewController() -> UIViewController? {
        let scenes = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
        let scene = scenes.first { $0.activationState == .foregroundActive } ?? scenes.first
        var top = scene?.keyWindow?.rootViewController
        while let presented = top?.presentedViewController, !presented.isBeingDismissed {
            top = presented
        }
        return top
    }

    private final class Coordinator: NSObject, UIDocumentPickerDelegate {
        private let done: (URL?) -> Void

        init(done: @escaping (URL?) -> Void) {
            self.done = done
        }

        func documentPicker(_: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
            self.done(urls.first)
        }

        func documentPickerWasCancelled(_: UIDocumentPickerViewController) {
            self.done(nil)
        }
    }
}

/// "Pick the folder: Dropbox › Dev › elovation", under the bar from the
/// moment Grant Folder Access is tapped until the picker returns: the
/// picker may open somewhere else, and this is where to go.
struct FolderPickerHint: View {
    let label: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Image(systemName: "folder.badge.plus").foregroundStyle(.tint)
            VStack(alignment: .leading, spacing: 2) {
                Text("Pick the folder: \(self.label)").font(.footnote.bold())
                Text("Go there if the picker opened elsewhere, then tap Open.").font(.caption)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        // Opaque: the page scrolls under the bar's inset.
        .background(.tint.opacity(0.12))
        .background(.background)
        .accessibilityElement(children: .combine)
    }
}
