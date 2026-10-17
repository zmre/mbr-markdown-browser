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
/// drew itself over anything placed beside it in the sheet. `directoryURL`
/// is still only a hint ("Picker will try to display this URL"), so Grant
/// Folder Access also names the folder in words (`FolderPickerHint`).
@MainActor
enum FolderPicker {
    /// The delegate of the picker on screen; UIKit holds delegates weakly.
    private static var current: Coordinator?

    /// Presents the picker. `done` gets the folder picked, or `nil` when it
    /// was cancelled (or could not be presented).
    ///
    /// With `hintHeight`, the sheet stops that far short of full height and
    /// leaves the screen above it undimmed, so the hint under the bar stays
    /// readable while picking.
    static func present(start: URL?, hintHeight: CGFloat = 0, done: @escaping (URL?) -> Void) {
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
        let set = picker.directoryURL?.path(percentEncoded: false) ?? "nil"
        Log
            .info(
                "picker: opening in \(start?.path(percentEncoded: false) ?? "the last place") (directoryURL now \(set))"
            )
        if hintHeight > 0, let sheet = picker.sheetPresentationController {
            let short = UISheetPresentationController.Detent.Identifier("mbr.belowHint")
            sheet.detents = [.custom(identifier: short) { $0.maximumDetentValue - hintHeight }]
            sheet.largestUndimmedDetentIdentifier = short
        }
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

/// "Pick the folder: Dropbox › Dev › elovation", above the picker: the
/// picker may open somewhere else, and this is where to go.
struct FolderPickerHint: View {
    /// The room `FolderPicker` leaves above its sheet for this.
    static let height: CGFloat = 130

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
