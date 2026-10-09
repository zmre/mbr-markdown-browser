//! mbr-ssg — mbr's static site generator.
//!
//! Renders a whole repository to HTML in parallel, writes the section pages,
//! `site.json` and the pagefind search index, and places the assets (`build`).
//! Built on `mbr-core`.

pub mod build;

pub use build::{BuildStats, Builder};
