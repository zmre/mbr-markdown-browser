//! Compiled-in default assets served under `/.mbr/*` and copied into static builds.
//!
//! Every asset is its own `pub const`, and [`DEFAULT_FILES`] is a table built
//! from them. The split exists for the consumers that need only a few of these
//! bytes: QuickLook inlines `theme.css` and mermaid into a preview and names
//! exactly those two items, not the whole table, so nothing it does not use has
//! a reason to be linked into it. The table is what the server's `/.mbr/*`
//! fallback and the static build's `.mbr/` step walk.
//!
//! Highlight.js and the default Pico stylesheet are owned by
//! [`crate::embedded_hljs`] and [`crate::embedded_pico`]; the table reuses their
//! constants rather than embedding the same files a second time.

use crate::{embedded_hljs as hljs, embedded_pico};

/// [`DEFAULT_FILES`] route of the lazy task-browser chunk.
///
/// Named because `build.rs` has to skip exactly this entry: the task browser is
/// server/GUI only (the index is built from live files), so shipping the chunk
/// into a static site would be dead weight behind a button that cannot exist.
pub const TASKS_CHUNK_ROUTE: &str = "/components/mbr-tasks.min.js";

/// [`DEFAULT_FILES`] route of the lazy review-notes chunk.
///
/// Skipped by `build.rs` for the same reason as [`TASKS_CHUNK_ROUTE`]: review
/// notes anchor to the `data-mbr-line` attributes only a server/GUI render
/// emits, so `<mbr-review>` never renders in a static site and the chunk would
/// be an unreachable payload in every generated page.
pub const REVIEW_CHUNK_ROUTE: &str = "/components/mbr-review.min.js";

/// [`DEFAULT_FILES`] route of the lazy search-panel extras chunk (folder picker,
/// note-type list).
///
/// Skipped by `build.rs` like [`TASKS_CHUNK_ROUTE`]: the controls it serves —
/// the scope select and the folder scope — render only in server/GUI mode,
/// because static search is Pagefind, which has neither facets nor folders.
pub const SEARCH_EXTRAS_CHUNK_ROUTE: &str = "/components/mbr-search-extras.min.js";

const JS: &str = "application/javascript";
const CSS: &str = "text/css";

pub const FAVICON_PNG: &[u8] = include_bytes!("../templates/favicon.png");
pub const THEME_CSS: &[u8] = include_bytes!("../templates/theme.css");
pub const USER_CSS: &[u8] = include_bytes!("../templates/user.css");

/// Main components bundle: every always-on element plus the lazy-chunk triggers.
pub const COMPONENTS_JS: &[u8] = include_bytes!("../templates/components-js/mbr-components.min.js");
/// Heavy Milkdown/Crepe editor chunk, lazy-loaded by `<mbr-editor>`.
pub const EDITOR_JS: &[u8] = include_bytes!("../templates/components-js/mbr-editor.min.js");
/// Sidebar mini force-graph chunk (d3-force), lazy-loaded by `<mbr-info>` when
/// the info panel first opens.
pub const GRAPH_JS: &[u8] = include_bytes!("../templates/components-js/mbr-graph.min.js");
/// Relationship-charts chunk (family-chart, timeline tree, org chart),
/// lazy-loaded by `<mbr-genealogy>` on person/organization pages only.
pub const GENEALOGY_JS: &[u8] = include_bytes!("../templates/components-js/mbr-genealogy.min.js");
/// Task-browser panel chunk, lazy-loaded by `<mbr-tasks>` the first time the
/// panel is opened. Excluded from static builds — see [`TASKS_CHUNK_ROUTE`].
pub const TASKS_JS: &[u8] = include_bytes!("../templates/components-js/mbr-tasks.min.js");
/// Review-notes panel and form, lazy-loaded by `<mbr-review>` the first time a
/// note is written or the list is opened. Excluded from static builds — see
/// [`REVIEW_CHUNK_ROUTE`].
pub const REVIEW_JS: &[u8] = include_bytes!("../templates/components-js/mbr-review.min.js");
/// Search-panel folder picker and note-type list, lazy-loaded by `<mbr-search>`
/// the first time the modal opens. Excluded from static builds — see
/// [`SEARCH_EXTRAS_CHUNK_ROUTE`].
pub const SEARCH_EXTRAS_JS: &[u8] =
    include_bytes!("../templates/components-js/mbr-search-extras.min.js");
/// Flashcard review overlay (+ ts-fsrs), lazy-loaded by `<mbr-flashcards>` when
/// a deck is opened. Ships in static builds: In order / Random review needs no
/// server, only spaced repetition does.
pub const FLASHCARDS_JS: &[u8] = include_bytes!("../templates/components-js/mbr-flashcards.min.js");
/// Flashcard reading view (collapsed review-history summaries), imported by
/// `<mbr-flashcards>` at idle on `type: flashcard` pages only. Separate from
/// the deck so reading a note never fetches ts-fsrs.
pub const FLASHCARDS_READING_JS: &[u8] =
    include_bytes!("../templates/components-js/mbr-flashcards-reading.min.js");

pub const MERMAID_JS: &[u8] = include_bytes!("../templates/mermaid.12.1.0.min.js");

// Reveal.js presentation framework
pub const REVEAL_JS: &[u8] = include_bytes!("../templates/reveal.6.0.2.js");
pub const REVEAL_CSS: &[u8] = include_bytes!("../templates/reveal.6.0.2.css");
pub const REVEAL_THEME_BLANK_CSS: &[u8] =
    include_bytes!("../templates/reveal.theme.blank.5.2.1.css");
pub const REVEAL_THEME_BLACK_CSS: &[u8] =
    include_bytes!("../templates/reveal.theme.black.5.2.1.css");
pub const REVEAL_THEME_WHITE_CSS: &[u8] =
    include_bytes!("../templates/reveal.theme.white.5.2.1.css");
pub const REVEAL_SLIDES_CSS: &[u8] = include_bytes!("../templates/reveal-slides.css");
pub const REVEAL_NOTES_JS: &[u8] = include_bytes!("../templates/reveal.notes.6.0.2.js");

/// Every compiled-in default asset as `(route, bytes, mime type)`.
///
/// Routes are relative to `/.mbr`. The order is the one the static build
/// writes them in.
pub const DEFAULT_FILES: &[(&str, &[u8], &str)] = &[
    ("/favicon.png", FAVICON_PNG, "image/png"),
    ("/theme.css", THEME_CSS, CSS),
    ("/user.css", USER_CSS, CSS),
    ("/pico.min.css", embedded_pico::PICO_DEFAULT, CSS),
    ("/components/mbr-components.min.js", COMPONENTS_JS, JS),
    ("/components/mbr-editor.min.js", EDITOR_JS, JS),
    ("/components/mbr-graph.min.js", GRAPH_JS, JS),
    ("/components/mbr-genealogy.min.js", GENEALOGY_JS, JS),
    (TASKS_CHUNK_ROUTE, TASKS_JS, JS),
    (REVIEW_CHUNK_ROUTE, REVIEW_JS, JS),
    (SEARCH_EXTRAS_CHUNK_ROUTE, SEARCH_EXTRAS_JS, JS),
    ("/components/mbr-flashcards.min.js", FLASHCARDS_JS, JS),
    (
        "/components/mbr-flashcards-reading.min.js",
        FLASHCARDS_READING_JS,
        JS,
    ),
    ("/hljs.dark.css", hljs::HLJS_DARK_CSS, CSS),
    ("/hljs.atom-one-dark.css", hljs::HLJS_ATOM_ONE_DARK_CSS, CSS),
    ("/hljs.js", hljs::HLJS_JS, JS),
    ("/hljs.lang.css.js", hljs::HLJS_LANG_CSS, JS),
    ("/hljs.lang.javascript.js", hljs::HLJS_LANG_JAVASCRIPT, JS),
    ("/hljs.lang.typescript.js", hljs::HLJS_LANG_TYPESCRIPT, JS),
    ("/hljs.lang.rust.js", hljs::HLJS_LANG_RUST, JS),
    ("/hljs.lang.python.js", hljs::HLJS_LANG_PYTHON, JS),
    ("/hljs.lang.bash.js", hljs::HLJS_LANG_BASH, JS),
    ("/hljs.lang.java.js", hljs::HLJS_LANG_JAVA, JS),
    ("/hljs.lang.scala.js", hljs::HLJS_LANG_SCALA, JS),
    ("/hljs.lang.go.js", hljs::HLJS_LANG_GO, JS),
    ("/hljs.lang.ruby.js", hljs::HLJS_LANG_RUBY, JS),
    ("/hljs.lang.nix.js", hljs::HLJS_LANG_NIX, JS),
    ("/hljs.lang.json.js", hljs::HLJS_LANG_JSON, JS),
    ("/hljs.lang.yaml.js", hljs::HLJS_LANG_YAML, JS),
    ("/hljs.lang.xml.js", hljs::HLJS_LANG_XML, JS),
    ("/hljs.lang.sql.js", hljs::HLJS_LANG_SQL, JS),
    ("/hljs.lang.dockerfile.js", hljs::HLJS_LANG_DOCKERFILE, JS),
    ("/hljs.lang.markdown.js", hljs::HLJS_LANG_MARKDOWN, JS),
    ("/mermaid.min.js", MERMAID_JS, JS),
    ("/reveal.js", REVEAL_JS, JS),
    ("/reveal.css", REVEAL_CSS, CSS),
    ("/reveal-theme-blank.css", REVEAL_THEME_BLANK_CSS, CSS),
    ("/reveal-theme-black.css", REVEAL_THEME_BLACK_CSS, CSS),
    ("/reveal-theme-white.css", REVEAL_THEME_WHITE_CSS, CSS),
    ("/reveal-slides.css", REVEAL_SLIDES_CSS, CSS),
    ("/reveal-notes.js", REVEAL_NOTES_JS, JS),
];

/// Looks up a compiled-in default asset by its route (relative to `/.mbr`).
///
/// Returns `(bytes, mime type)`, or `None` when no default exists for `route`.
#[must_use]
pub fn default_file(route: &str) -> Option<(&'static [u8], &'static str)> {
    DEFAULT_FILES
        .iter()
        .find(|(name, _, _)| *name == route)
        .map(|(_, bytes, mime)| (*bytes, *mime))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn routes_are_unique_and_rooted() {
        let mut seen = HashSet::new();
        for (route, _, _) in DEFAULT_FILES {
            assert!(route.starts_with('/'), "route {route} must start with /");
            assert!(seen.insert(*route), "duplicate route {route}");
        }
    }

    #[test]
    fn default_file_finds_every_table_entry() {
        for (route, bytes, mime) in DEFAULT_FILES {
            assert_eq!(default_file(route), Some((*bytes, *mime)));
        }
        assert_eq!(default_file("/nope.js"), None);
    }

    #[test]
    fn chunk_routes_are_in_the_table() {
        for route in [
            TASKS_CHUNK_ROUTE,
            REVIEW_CHUNK_ROUTE,
            SEARCH_EXTRAS_CHUNK_ROUTE,
        ] {
            assert!(default_file(route).is_some(), "{route} missing");
        }
    }

    #[test]
    fn quicklook_assets_match_their_routes() {
        assert_eq!(default_file("/theme.css").map(|(b, _)| b), Some(THEME_CSS));
        assert_eq!(
            default_file("/mermaid.min.js").map(|(b, _)| b),
            Some(MERMAID_JS)
        );
    }
}
