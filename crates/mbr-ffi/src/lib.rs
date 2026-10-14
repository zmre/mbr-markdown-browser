//! mbr-ffi — the UniFFI surface the Apple Swift shells link.
//!
//! Built as the `libmbr_ffi.a` staticlib that the QuickLook extension
//! (`apple/quicklook`) links with `-lmbr_ffi`; the Swift bindings in
//! `apple/quicklook/Generated` are generated from it. Depends on `mbr-core`
//! alone — no server, watcher, static site generator, CLI, GUI or ffmpeg, none
//! of which a sandboxed preview can use.
//!
//! With the `mobile` feature it is also the iOS app's core ([`mobile`]): the
//! in-process server behind the app's `mbr://` scheme handler, the coordinated
//! vault, the persistent index. That surface uses UniFFI proc-macros beside the
//! QuickLook UDL; one Swift module carries both.
//!
//! The UniFFI exports are Apple targets only. Elsewhere QuickLook compiles to
//! nothing and `mobile` compiles as plain Rust (so its tests run anywhere),
//! and `cargo build --workspace` works without the Swift bindings generator.

// UniFFI's generated UDL scaffolding (`OUT_DIR/mbr.uniffi.rs`) holds its
// interface metadata in one large `const` byte array
// (`UNIFFI_META_CONST_UDL_MBR`), which trips `clippy::large_const_arrays`.
// The generated code is not ours to change, and an `#[allow]` on the
// `include_scaffolding!` call is discarded as an unused attribute. Wrapping
// the include in a module would scope the allow but move `UniFfiTag` off the
// crate root, where UniFFI's proc-macro exports look for it. So the allow is
// crate-wide.
#![allow(clippy::large_const_arrays)]

// The scaffolding must be included at the crate root for UniFFI to work.
#[cfg(target_vendor = "apple")]
uniffi::include_scaffolding!("mbr");

#[cfg(target_vendor = "apple")]
pub mod quicklook;

#[cfg(feature = "mobile")]
pub mod mobile;

#[cfg(target_vendor = "apple")]
pub use quicklook::{
    PreviewAttachment, PreviewDocument, QuickLookConfig, QuickLookError, find_config_root,
    render_preview, render_preview_with_config,
};
