//! mbr-ffi — the UniFFI surface the Apple Swift shells link.
//!
//! Built as the `libmbr_ffi.a` staticlib that the QuickLook extension
//! (`apple/quicklook`) links with `-lmbr_ffi`; the Swift bindings in
//! `apple/quicklook/Generated` are generated from it. Depends on `mbr-core`
//! alone — no server, watcher, static site generator, CLI, GUI or ffmpeg, none
//! of which a sandboxed preview can use.
//!
//! Apple targets only. On every other platform this crate compiles to nothing,
//! so `cargo build --workspace` works everywhere without building the Swift
//! bindings generator.

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

#[cfg(target_vendor = "apple")]
pub use quicklook::{
    PreviewAttachment, PreviewDocument, QuickLookConfig, QuickLookError, find_config_root,
    render_preview, render_preview_with_config,
};
