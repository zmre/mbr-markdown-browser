//! Picking a folder or a markdown file to open.
//!
//! Shared by the initial-launch picker in `main.rs` (shown when there is no
//! meaningful path to open — see `main::needs_folder_picker`) and the
//! "Open…" command / Cmd+O in `browser.rs`, so the two entry points cannot
//! answer "what can be picked" differently.
//!
//! macOS gets one dialog that returns either. rfd 0.17.2 already builds
//! exactly that panel for macOS — an `NSOpenPanel` with `canChooseFiles` and
//! `canChooseDirectories` both `true`, the same configuration Xcode's own
//! Open dialog uses — as `FileDialog::pick_file_or_folder`
//! (`rfd::backend::macos::file_dialog::panel_ffi::build_pick_file_or_folder`),
//! so this calls straight into it instead of re-deriving the same AppKit
//! calls by hand. That also sidesteps having to reason about main-thread
//! dispatch ourselves: rfd's synchronous pickers already funnel through
//! `dispatch2::run_on_main`, safe to call from any thread, which is why this
//! function makes no thread-affinity promise of its own — `main.rs` calls it
//! before the event loop exists, `browser.rs` from a spawned background
//! thread, and both already work today with the plain `pick_folder()` this
//! replaces.
//!
//! Windows and Linux have no native mixed picker, so there a small two-button
//! prompt asks which kind of dialog to open first, then rfd's separate
//! `pick_folder`/`pick_file` runs exactly as it always has.
//!
//! That prompt is plain `MessageButtons::YesNo`, not `OkCancelCustom` — rfd
//! only renders custom button *labels* through `TaskDialogIndirect`, gated
//! behind its `common-controls-v6` Cargo feature, and that function is
//! exported only by the side-by-side, manifest-activated v6 `comctl32.dll`.
//! A Rust binary embeds no such manifest by default, so the statically
//! linked import resolves against the older `comctl32.dll` every Windows
//! ships for compatibility — which does not export `TaskDialogIndirect` at
//! all. That is not a graceful degrade to plain buttons; it is
//! `STATUS_ENTRYPOINT_NOT_FOUND` at process launch, for every invocation of
//! the binary that reaches this code in its static call graph, including one
//! that never shows the dialog — confirmed the hard way, as two Windows CI
//! legs failing every `cli_integration.rs` subprocess test with that exact
//! NTSTATUS the first time this feature was enabled. `YesNo` has called
//! `MessageBoxW` unconditionally since Windows 1.0, so it has no comctl32
//! version to get wrong.

use std::path::PathBuf;

/// Ask the user to choose a markdown file or a folder to open.
///
/// `markdown_extensions` limits which files are enabled/selectable in the
/// dialog; pass `Config::default().markdown_extensions` where no repository
/// config has been loaded yet (the very first launch, before `Config::read`
/// has run).
pub fn pick_file_or_folder(title: &str, markdown_extensions: &[String]) -> Option<PathBuf> {
    let dialog = rfd::FileDialog::new()
        .set_title(title)
        .add_filter("Markdown", markdown_extensions);

    #[cfg(target_os = "macos")]
    {
        dialog.pick_file_or_folder()
    }

    #[cfg(not(target_os = "macos"))]
    {
        match prompt_open_kind(title)? {
            OpenKind::Folder => dialog.pick_folder(),
            OpenKind::File => dialog.pick_file(),
        }
    }
}

/// Which kind of dialog to show next, on platforms with no mixed picker.
#[cfg(not(target_os = "macos"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenKind {
    Folder,
    File,
}

/// Asks whether to open a folder or a file. `None` means the user cancelled.
#[cfg(not(target_os = "macos"))]
fn prompt_open_kind(title: &str) -> Option<OpenKind> {
    let result = rfd::MessageDialog::new()
        .set_title(title)
        .set_description(
            "Open a folder to browse? Choose \"No\" to pick a single markdown file instead.",
        )
        .set_buttons(rfd::MessageButtons::YesNo)
        .show();

    interpret_open_kind(result)
}

/// Pure match behind [`prompt_open_kind`], split out so it is testable
/// without a real dialog.
#[cfg(not(target_os = "macos"))]
fn interpret_open_kind(result: rfd::MessageDialogResult) -> Option<OpenKind> {
    match result {
        rfd::MessageDialogResult::Yes => Some(OpenKind::Folder),
        rfd::MessageDialogResult::No => Some(OpenKind::File),
        _ => None,
    }
}

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn yes_opens_a_folder() {
        assert_eq!(
            interpret_open_kind(rfd::MessageDialogResult::Yes),
            Some(OpenKind::Folder)
        );
    }

    #[test]
    fn no_opens_a_file() {
        assert_eq!(
            interpret_open_kind(rfd::MessageDialogResult::No),
            Some(OpenKind::File)
        );
    }

    #[test]
    fn cancel_and_unrecognized_results_cancel_the_picker() {
        assert_eq!(interpret_open_kind(rfd::MessageDialogResult::Cancel), None);
        assert_eq!(interpret_open_kind(rfd::MessageDialogResult::Ok), None);
        assert_eq!(
            interpret_open_kind(rfd::MessageDialogResult::Custom("unexpected".to_string())),
            None
        );
    }
}
