import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// The system folder picker, opening in `start` when given.
///
/// `UIDocumentPickerViewController` rather than `.fileImporter`: on a device,
/// `.fileDialogDefaultDirectory` was ignored for a file opened from a
/// provider (Grant Folder Access started somewhere else), while the picker's
/// own `directoryURL` is the documented way to choose where it opens.
struct FolderPicker: UIViewControllerRepresentable {
    /// Where the picker opens; `nil` for wherever it was last.
    let start: URL?
    /// The folder picked, or `nil` when the picker was cancelled.
    let done: (URL?) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(done: self.done)
    }

    func makeUIViewController(context: Context) -> UIDocumentPickerViewController {
        let picker = UIDocumentPickerViewController(forOpeningContentTypes: [.folder], asCopy: false)
        picker.allowsMultipleSelection = false
        picker.directoryURL = self.start
        picker.delegate = context.coordinator
        if let start {
            Log.info("picker: opening in \(start.path(percentEncoded: false))")
        }
        return picker
    }

    func updateUIViewController(_: UIDocumentPickerViewController, context _: Context) {}

    final class Coordinator: NSObject, UIDocumentPickerDelegate {
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
