//! Repository-wide broken-link report: `mbr --check-links-only`.
//!
//! Scans a repository once, renders every note the way the server renders a
//! page for its `errors.json` (oembed off, nothing written), and reports what
//! the server's errors panel would report for that page — broken `<a href>`s
//! ([`page_errors::rendered_link_defects`]), broken media
//! ([`page_errors::validate_media_references`] and a deck's default
//! background), unresolved and ambiguous wikilinks — so the report and the
//! panel can never disagree about whether a link is broken.
//!
//! Two things the panel does not do:
//!
//! - **Source lines.** The validators read the *emitted* HTML, which has no
//!   source positions. [`scan_occurrences`] re-parses the note with the
//!   renderer's own parser setup and computes, for every link, image and raw
//!   HTML `href`/`src`/`poster`, the href the renderer emits for it — through
//!   the same functions the renderer calls ([`crate::markdown::link_href`],
//!   [`transform_asset_url`]) — so a broken emitted href maps back to every
//!   line that produced it. Something no occurrence explains (a link inside a
//!   chat bubble, a `--- {data-background-image=…}` section) falls back to the
//!   lines naming the target's file name, else is reported without a line.
//! - **Suggestions.** [`crate::not_found`] — the same matcher as the 404 page's
//!   "where did it go" — finds where a missing target probably went, and the
//!   replacement is written in the style the author used (see
//!   [`DestinationStyle`]).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pulldown_cmark::{Event, LinkType, Tag};
use serde::Serialize;

use crate::config::Config;
use crate::link_index::{resolve_relative_url, resolve_relative_url_checked};
use crate::link_transform::{LinkTransformConfig, transform_asset_url, transform_link};
use crate::markdown::{self, RenderOptions, ReviewLines};
use crate::not_found::{MAX_DID_YOU_MEAN, NotFoundIndex, SiteNote, SuggestedPage, SuggestionKind};
use crate::outside_link::OutsideLinkContext;
use crate::page_errors::{self, LinkDefect, MediaKind, PageError};
use crate::path_resolver::{OwnedPathResolverConfig, PathResolverConfig};
use crate::repo::{MarkdownInfo, Repo};
use crate::wikilink_index::WikilinkIndex;

/// Version of the JSON report's shape. Bumped on any incompatible change.
pub const REPORT_SCHEMA_VERSION: u32 = 1;

/// Longest `authored` excerpt kept, in characters.
const MAX_AUTHORED_CHARS: usize = 160;

// ---------------------------------------------------------------------------
// The report
// ---------------------------------------------------------------------------

/// The whole report. Serialized as the `--format json` output.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LinkReport {
    /// [`REPORT_SCHEMA_VERSION`].
    pub schema_version: u32,
    pub summary: Summary,
    /// Notes with at least one problem, by path.
    pub files: Vec<FileProblems>,
    /// Notes that could not be read (not downloaded yet, unreadable), by path.
    pub unreadable: Vec<UnreadableFile>,
}

/// Counts for the report's heading.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
pub struct Summary {
    /// Notes rendered and checked.
    pub files_checked: usize,
    /// Notes with at least one problem.
    pub files_with_problems: usize,
    /// Problems in all notes.
    pub problems: usize,
    /// Notes that could not be read.
    pub unreadable: usize,
}

/// The problems of one note.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FileProblems {
    /// Repository-relative source path, `/`-separated.
    pub path: String,
    /// The note's site URL.
    pub url: String,
    /// By line (unknown lines last), then kind, then href.
    pub problems: Vec<Problem>,
}

/// A note that could not be checked.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UnreadableFile {
    pub path: String,
    pub error: String,
}

/// What kind of reference is broken.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKind {
    /// An `<a href>`: a markdown link, a wikilink that did not resolve, a raw
    /// HTML link.
    BrokenLink,
    /// An `<img>`, `<video>`, `<audio>` or `<source>` `src`, or a slide
    /// background.
    BrokenMedia,
    /// A literal `[[…]]` the renderer could not turn into a link.
    UnresolvedWikilink,
    /// A `[[Name]]` several notes answer to; it resolved to one of them.
    AmbiguousWikilink,
}

/// How the reference was written.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Syntax {
    /// `[text](dest)`, `<url>`, `[text][ref]`.
    Link,
    /// `[[Name]]`, `[[Name|Text]]`.
    Wikilink,
    /// `![alt](dest)`.
    Image,
    /// A raw HTML attribute (`<a href>`, `<img src>`, `poster`).
    Html,
}

/// Why it is broken.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// Nothing is served at the target.
    Missing,
    /// A `..` climbs above the repository root.
    EscapesRoot,
    /// A link to a markdown page without its trailing slash.
    MissingTrailingSlash,
    /// A literal `[[…]]` was left in the page.
    Unresolved,
    /// Several notes answer to the name.
    Ambiguous,
}

/// One broken reference, at one place in the source.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Problem {
    /// 1-based line in the file (frontmatter included); `null` when it could
    /// not be located.
    pub line: Option<usize>,
    pub kind: ProblemKind,
    pub reason: Reason,
    /// How it was written; `null` when it was located only by its target's
    /// file name, or not at all.
    pub syntax: Option<Syntax>,
    /// For [`ProblemKind::BrokenMedia`]: which element.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<MediaKind>,
    /// The construct as written (`[Setup](../setup.md)`, `[[Old Name]]`,
    /// `<img src="x.png">`), or the source line when only the line is known.
    pub authored: Option<String>,
    /// The destination as written (`../setup.md`, `Old Name`).
    pub destination: Option<String>,
    /// The link's text in the page, when it is a link.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The href or src the renderer emitted (what a reader's browser follows).
    pub href: Option<String>,
    /// The site URL the reference resolves to, decoded (`/setup/`).
    pub target: Option<String>,
    /// Up to three replacements, best first. Empty when nothing matched.
    pub suggestions: Vec<Suggestion>,
    /// When nothing matched: the missing target's folder still exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<FolderHint>,
    /// For [`ProblemKind::AmbiguousWikilink`]: the note it resolved to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_to: Option<String>,
    /// For [`ProblemKind::AmbiguousWikilink`]: every note answering to the
    /// name (the one it resolved to first), each as an unambiguous
    /// replacement.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<Suggestion>,
}

/// A replacement for a broken reference.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Suggestion {
    /// Ready to paste in place of the destination, in the author's style
    /// (relative path, root-relative URL, `[[Name]]`). For a wikilink it is
    /// the whole `[[…]]`.
    pub replacement: String,
    /// The target's site URL.
    pub url: String,
    /// The target's repository-relative path, when it is in the repository.
    pub path: Option<String>,
    /// The target's title (frontmatter title, else its name).
    pub title: Option<String>,
}

/// The surviving folder of a missing target.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FolderHint {
    pub url: String,
    pub title: String,
}

impl LinkReport {
    /// Whether anything is broken. Unreadable notes are reported but do not
    /// count.
    pub fn has_problems(&self) -> bool {
        self.summary.problems > 0
    }

    /// The report as pretty-printed JSON (the `--format json` output).
    pub fn to_json(&self) -> String {
        // Plain strings, numbers and enums: serialization cannot fail.
        serde_json::to_string_pretty(self).unwrap_or_else(|e| format!("{{\"error\": \"{e}\"}}"))
    }

    /// The report as a markdown todo list: one `##` section per note, one
    /// checkbox per broken reference with its suggestions nested under it.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        let Summary {
            files_checked,
            files_with_problems,
            problems,
            unreadable,
        } = self.summary;
        if problems == 0 {
            out.push_str(&format!(
                "No broken links in {}.\n",
                plural(files_checked, "file", "files")
            ));
        } else {
            out.push_str(&format!(
                "# Broken links: {} in {}\n",
                problems,
                plural(files_with_problems, "file", "files")
            ));
        }
        for file in &self.files {
            out.push_str(&format!("\n## {}\n\n", file.path));
            for problem in &file.problems {
                push_markdown_problem(&mut out, problem);
            }
        }
        if unreadable > 0 {
            out.push_str("\n## Could not be checked\n\n");
            for file in &self.unreadable {
                out.push_str(&format!("- {} — {}\n", code(&file.path), file.error));
            }
        }
        out
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `s` as markdown inline code, whatever backticks it holds.
fn code(s: &str) -> String {
    let longest = s.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest + 1);
    if longest > 0 || s.starts_with(' ') || s.ends_with(' ') {
        format!("{fence} {s} {fence}")
    } else {
        format!("{fence}{s}{fence}")
    }
}

fn push_markdown_problem(out: &mut String, problem: &Problem) {
    let line = problem
        .line
        .map_or_else(String::new, |line| format!("L{line} "));
    let what = problem
        .authored
        .as_deref()
        .or(problem.href.as_deref())
        .map_or_else(String::new, |s| format!("{} ", code(s)));
    let reason = match (problem.kind, problem.reason) {
        (_, Reason::EscapesRoot) => "climbs above the repository root".to_string(),
        (_, Reason::MissingTrailingSlash) => "page link without its trailing slash".to_string(),
        (ProblemKind::BrokenMedia, _) => "no such file".to_string(),
        (ProblemKind::UnresolvedWikilink, _) => "unresolved wikilink".to_string(),
        (ProblemKind::AmbiguousWikilink, _) => format!(
            "ambiguous wikilink: {} notes answer to it; it opens {}",
            problem.candidates.len(),
            code(problem.resolved_to.as_deref().unwrap_or("?"))
        ),
        (ProblemKind::BrokenLink, _) if problem.syntax == Some(Syntax::Wikilink) => {
            "no note by that name".to_string()
        }
        (ProblemKind::BrokenLink, _) => "no such page or file".to_string(),
    };
    out.push_str(&format!("- [ ] {line}{what}— {reason}\n"));
    for suggestion in &problem.suggestions {
        out.push_str(&format!(
            "  - did you mean: {}{}\n",
            code(&suggestion.replacement),
            suggestion
                .title
                .as_deref()
                .map_or_else(String::new, |title| format!(" ({title})"))
        ));
    }
    for candidate in &problem.candidates {
        out.push_str(&format!(
            "  - candidate: {} ({})\n",
            code(&candidate.replacement),
            candidate.url
        ));
    }
    if let Some(folder) = &problem.folder {
        out.push_str(&format!(
            "  - no match; folder {} still exists\n",
            code(&folder.url)
        ));
    }
}

// ---------------------------------------------------------------------------
// Checking a repository
// ---------------------------------------------------------------------------

/// Everything per-note checking shares, built once per run.
pub struct LinkChecker {
    repo: Arc<Repo>,
    root_dir: PathBuf,
    markdown_extensions: Vec<String>,
    index_file: String,
    resolver: OwnedPathResolverConfig,
    directory_url_probe: crate::link_transform::DirectoryUrlProbe,
    outside_links: OutsideLinkContext,
    valid_tag_sources: HashSet<String>,
    suggestions: NotFoundIndex,
    /// Repository-relative paths of the files in `suggestions`' file list,
    /// in its order; `None` for a file outside the repository (an external
    /// static overlay).
    file_paths: Vec<Option<String>>,
}

impl LinkChecker {
    /// Scans the repository `config` describes (ignore rules, static overlay,
    /// symlink mounts and hidden-file rules as the server applies them) and
    /// builds the indexes checking needs.
    pub fn scan(config: &Config) -> Result<Self, crate::errors::RepoError> {
        let repo = Repo::init_from_config(config);
        repo.scan_all()?;
        repo.scan_static_folder()?;
        repo.build_wikilink_index();
        Ok(Self::new(Arc::new(repo), config))
    }

    /// The checker for an already scanned repository.
    pub fn new(repo: Arc<Repo>, config: &Config) -> Self {
        let resolver = OwnedPathResolverConfig::for_repo(
            &repo,
            &config.static_folder,
            &config.markdown_extensions,
            &config.index_file,
            crate::config::tag_sources_to_url_sources(&config.tag_sources),
        );
        let directory_url_probe =
            crate::link_transform::filesystem_directory_url_probe(resolver.clone());
        // The server's context, so a page renders exactly as it does there.
        let outside_links = OutsideLinkContext::for_repo(&repo).naming_absolute_paths();

        let mut notes: Vec<SiteNote> = repo
            .markdown_files
            .pin()
            .iter()
            .map(|(_, info)| SiteNote {
                url_path: info.url_path.clone(),
                raw_path: Some(crate::url_path::path_to_url(&info.raw_path)),
                title: info
                    .frontmatter
                    .as_ref()
                    .and_then(|fm| fm.get("title"))
                    .and_then(|title| title.as_str())
                    .map(str::to_string),
            })
            .collect();
        notes.sort_by(|a, b| {
            a.url_path
                .cmp(&b.url_path)
                .then_with(|| a.raw_path.cmp(&b.raw_path))
        });
        let mut files: Vec<(String, Option<String>)> = repo
            .other_files
            .pin()
            .iter()
            .map(|(key, info)| {
                let path = repo
                    .vault()
                    .vault_path(key)
                    .ok()
                    .map(|path| path.as_str().to_string());
                (info.url_path.clone(), path)
            })
            .collect();
        files.sort();
        let suggestions = NotFoundIndex::new(
            notes,
            files.iter().map(|(url, _)| url.clone()),
            &config.index_file,
        );

        Self {
            root_dir: config.root_dir.clone(),
            markdown_extensions: config.markdown_extensions.clone(),
            index_file: config.index_file.clone(),
            resolver,
            directory_url_probe,
            outside_links,
            valid_tag_sources: crate::config::tag_sources_to_set(&config.tag_sources),
            suggestions,
            file_paths: files.into_iter().map(|(_, path)| path).collect(),
            repo,
        }
    }

    /// How many notes [`Self::check_all`] will check.
    pub fn note_count(&self) -> usize {
        self.repo.markdown_files.pin().len()
    }

    /// Checks every note, `concurrency` at a time (`None`: rayon's default),
    /// and assembles the report in a deterministic order.
    pub fn check_all(&self, concurrency: Option<usize>) -> LinkReport {
        use rayon::prelude::*;

        let mut notes: Vec<(PathBuf, MarkdownInfo)> = self
            .repo
            .markdown_files
            .pin()
            .iter()
            .map(|(key, info)| (key.clone(), info.clone()))
            .collect();
        notes.sort_by(|a, b| a.1.raw_path.cmp(&b.1.raw_path));

        let check = || -> Vec<NoteOutcome> {
            notes
                .par_iter()
                .map(|(key, info)| self.check_note(key, info))
                .collect()
        };
        let outcomes = match concurrency.and_then(|n| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(n.max(1))
                .build()
                .ok()
        }) {
            Some(pool) => pool.install(check),
            None => check(),
        };

        let mut report = LinkReport {
            schema_version: REPORT_SCHEMA_VERSION,
            summary: Summary {
                files_checked: notes.len(),
                ..Summary::default()
            },
            files: Vec::new(),
            unreadable: Vec::new(),
        };
        for outcome in outcomes {
            match outcome {
                NoteOutcome::Checked(file) if file.problems.is_empty() => {}
                NoteOutcome::Checked(file) => {
                    report.summary.problems += file.problems.len();
                    report.files.push(file);
                }
                NoteOutcome::Unreadable(file) => report.unreadable.push(file),
            }
        }
        report.files.sort_by(|a, b| a.path.cmp(&b.path));
        report.unreadable.sort_by(|a, b| a.path.cmp(&b.path));
        report.summary.files_with_problems = report.files.len();
        report.summary.unreadable = report.unreadable.len();
        report.summary.files_checked -= report.summary.unreadable;
        report
    }

    fn link_transform_config(&self, key: &Path, url_path: &str) -> LinkTransformConfig {
        LinkTransformConfig {
            markdown_extensions: self.markdown_extensions.clone(),
            index_file: self.index_file.clone(),
            is_index_file: key
                .file_name()
                .and_then(|f| f.to_str())
                .is_some_and(|f| f == self.index_file),
            // Server mode: root-relative URLs stay root-relative.
            url_depth: None,
            current_page_url: url_path.to_string(),
            directory_url_probe: Some(Arc::clone(&self.directory_url_probe)),
            outside_links: Some(self.outside_links.clone()),
        }
    }

    /// Renders one note as the server's `errors.json` does and reports its
    /// broken references, each located in the source.
    fn check_note(&self, key: &Path, info: &MarkdownInfo) -> NoteOutcome {
        let path = crate::url_path::path_to_url(&info.raw_path);
        let source = match markdown::read_source(self.repo.vault().as_ref(), key) {
            Ok(source) => source,
            Err(e) => {
                return NoteOutcome::Unreadable(UnreadableFile {
                    path,
                    error: error_message(&e),
                });
            }
        };
        let page_url = info.url_path.as_str();
        let link_config = self.link_transform_config(key, page_url);
        let wikilink_index = Arc::clone(&self.repo.wikilink_index);
        let rendered = match markdown::render_str(
            &source,
            key,
            RenderOptions {
                root_path: &self.root_dir,
                oembed_timeout_ms: 0,
                link_transform_config: link_config.clone(),
                oembed_cache: None,
                // As the server renders a page (and its errors.json).
                server_mode: true,
                transcode_enabled: false,
                valid_tag_sources: self.valid_tag_sources.clone(),
                review: ReviewLines::Omit,
                mark_incomplete: false,
                incomplete_markers: &[],
                wikilink_index: Some(Arc::clone(&wikilink_index)),
                math_fences: true,
            },
        ) {
            Ok(rendered) => rendered,
            Err(e) => {
                return NoteOutcome::Unreadable(UnreadableFile {
                    path,
                    error: error_message(&e),
                });
            }
        };

        let resolver = self.resolver.as_config();
        let found = Found::collect(&rendered, &resolver, page_url);
        if found.is_empty() {
            return NoteOutcome::Checked(FileProblems {
                path,
                url: info.url_path.clone(),
                problems: Vec::new(),
            });
        }

        let prepared = markdown::prepare_source(&source, &self.valid_tag_sources);
        let located = Located::scan(
            &prepared,
            &link_config,
            &self.valid_tag_sources,
            Some(&wikilink_index),
        );
        let context = NoteContext {
            checker: self,
            source_path: &path,
            page_url,
            located: &located,
        };
        let mut problems = context.problems(found);
        problems.sort_by(|a, b| {
            (a.line.is_none(), a.line, a.kind, &a.href, &a.authored).cmp(&(
                b.line.is_none(),
                b.line,
                b.kind,
                &b.href,
                &b.authored,
            ))
        });
        NoteOutcome::Checked(FileProblems {
            path,
            url: info.url_path.clone(),
            problems,
        })
    }

    /// The styled replacement for one suggested target.
    fn suggestion(
        &self,
        page: &SuggestedPage,
        style: &DestinationStyle,
        source_path: &str,
    ) -> Suggestion {
        let (path, is_index) = match page.kind {
            SuggestionKind::Note => {
                let path = self
                    .suggestions
                    .note(page.index)
                    .and_then(|note| note.raw_path.clone());
                let is_index = path
                    .as_deref()
                    .is_some_and(|p| file_name(p) == self.index_file);
                (path, is_index)
            }
            SuggestionKind::File => (self.file_paths.get(page.index).cloned().flatten(), false),
        };
        let replacement = style.write(&Target {
            url: &page.url,
            path: path.as_deref(),
            kind: page.kind,
            is_index,
            unique_name: page.kind == SuggestionKind::Note
                && path
                    .as_deref()
                    .is_some_and(|p| self.suggestions.notes_named(stem(file_name(p))) == 1),
            source_path,
        });
        Suggestion {
            replacement,
            url: page.url.clone(),
            path,
            title: (page.kind == SuggestionKind::Note).then(|| page.title.clone()),
        }
    }
}

/// The message of an error and its sources, on one line.
fn error_message(error: &dyn std::error::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !message.contains(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        source = cause.source();
    }
    message
}

enum NoteOutcome {
    Checked(FileProblems),
    Unreadable(UnreadableFile),
}

// ---------------------------------------------------------------------------
// What the validators found in the rendered page
// ---------------------------------------------------------------------------

/// The page's problems, as the server's validators report them.
struct Found {
    links: Vec<page_errors::BrokenRenderedLink>,
    /// `(src, kind)`, deduplicated.
    media: Vec<(String, MediaKind)>,
    /// Literal `[[…]]` left in the page.
    unresolved: Vec<String>,
    ambiguous: Vec<crate::wikilink_index::AmbiguousWikilink>,
}

impl Found {
    fn collect(
        rendered: &markdown::MarkdownRenderResult,
        resolver: &PathResolverConfig<'_>,
        page_url: &str,
    ) -> Self {
        let html = &rendered.html;
        let mut seen = HashSet::new();
        let background = rendered
            .slides
            .as_ref()
            .and_then(|slides| slides.settings.background_image.as_deref())
            .and_then(|src| page_errors::validate_slide_background(src, resolver, page_url));
        let media = page_errors::validate_media_references(html, resolver, page_url)
            .into_iter()
            .chain(background)
            .filter_map(|error| match error {
                PageError::BrokenMediaReference { src, kind } => Some((src, kind)),
                _ => None,
            })
            .filter(|(src, _)| seen.insert(src.clone()))
            .collect();
        let unresolved = page_errors::detect_unresolved_wikilinks(html)
            .into_iter()
            .filter_map(|error| match error {
                PageError::UnresolvedWikilink { raw } => Some(raw),
                _ => None,
            })
            .collect();
        Self {
            links: page_errors::rendered_link_defects(html, resolver, page_url),
            media,
            unresolved,
            ambiguous: rendered.ambiguous_wikilinks.clone(),
        }
    }

    fn is_empty(&self) -> bool {
        self.links.is_empty()
            && self.media.is_empty()
            && self.unresolved.is_empty()
            && self.ambiguous.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Locating references in the source
// ---------------------------------------------------------------------------

/// Which construct a reference was written as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Construct {
    /// `[text](dest)`, `<url>`, `[text][ref]`.
    Link,
    /// `[[Name]]`, `[[Name|Text]]`.
    Wikilink,
    /// `![alt](dest)`.
    Image,
    /// A raw HTML `href`.
    HtmlHref,
    /// A raw HTML `src` / `poster`.
    HtmlSrc,
}

impl Construct {
    fn syntax(self) -> Syntax {
        match self {
            Self::Link => Syntax::Link,
            Self::Wikilink => Syntax::Wikilink,
            Self::Image => Syntax::Image,
            Self::HtmlHref | Self::HtmlSrc => Syntax::Html,
        }
    }
}

/// One reference in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Occurrence {
    line: usize,
    construct: Construct,
    /// The construct as written (whitespace runs collapsed, shortened).
    authored: String,
    /// The destination as written.
    destination: String,
    /// The display part of a piped wikilink (`Text` in `[[Name|Text]]`).
    wikilink_text: Option<String>,
    /// [`href_key`] of what the renderer emits for it.
    key: String,
}

/// Every reference in a note, with the href the renderer emits for each.
struct Located<'s> {
    occurrences: Vec<Occurrence>,
    source: &'s str,
    line_starts: Vec<usize>,
}

/// A comparable form of an href: percent-decoded, entities decoded. The
/// emitted HTML is percent-escaped by the writer (`escape_href`) and the
/// validators read attribute values with entities decoded, so neither
/// spelling can be compared byte for byte with the transform's output.
fn href_key(href: &str) -> String {
    let decoded = html_escape::decode_html_entities(href);
    percent_encoding::percent_decode_str(&decoded)
        .decode_utf8_lossy()
        .into_owned()
}

fn excerpt(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > MAX_AUTHORED_CHARS {
        let cut: String = collapsed.chars().take(MAX_AUTHORED_CHARS - 1).collect();
        format!("{cut}…")
    } else {
        collapsed
    }
}

impl<'s> Located<'s> {
    /// Parses `source` — the note after [`markdown::prepare_source`], whose
    /// line breaks are the file's — with the renderer's parser setup
    /// ([`crate::frontmatter::parse_with_offsets`] and
    /// [`markdown::markdown_options`]) and records every link, image and raw
    /// HTML URL attribute with the href the renderer writes for it.
    fn scan(
        source: &'s str,
        config: &LinkTransformConfig,
        valid_tag_sources: &HashSet<String>,
        wikilink_index: Option<&WikilinkIndex>,
    ) -> Self {
        let line_starts: Vec<usize> = std::iter::once(0)
            .chain(source.match_indices('\n').map(|(at, _)| at + 1))
            .collect();
        let line_of = |offset: usize| line_starts.partition_point(|&start| start <= offset);
        let mut occurrences = Vec::new();

        for (event, range) in
            crate::frontmatter::parse_with_offsets(source, markdown::markdown_options())
        {
            let authored = || excerpt(source.get(range.clone()).unwrap_or_default());
            match &event {
                Event::Start(Tag::Link {
                    link_type: LinkType::Email,
                    ..
                }) => {}
                Event::Start(Tag::Link {
                    link_type,
                    dest_url,
                    ..
                }) => {
                    let is_wikilink = matches!(link_type, LinkType::WikiLink { .. });
                    let href = markdown::link_href(
                        dest_url,
                        is_wikilink,
                        config,
                        valid_tag_sources,
                        wikilink_index,
                    )
                    .href;
                    let authored = authored();
                    occurrences.push(Occurrence {
                        line: line_of(range.start),
                        construct: if is_wikilink {
                            Construct::Wikilink
                        } else {
                            Construct::Link
                        },
                        wikilink_text: is_wikilink.then(|| wikilink_display(&authored)).flatten(),
                        authored,
                        destination: dest_url.to_string(),
                        key: href_key(&href),
                    });
                }
                Event::Start(Tag::Image { dest_url, .. }) => occurrences.push(Occurrence {
                    line: line_of(range.start),
                    construct: Construct::Image,
                    authored: authored(),
                    destination: dest_url.to_string(),
                    wikilink_text: None,
                    key: href_key(&transform_asset_url(dest_url, config)),
                }),
                Event::Html(html) | Event::InlineHtml(html) => {
                    let first_line = line_of(range.start);
                    for caps in markdown::HTML_URL_ATTR.captures_iter(html) {
                        let (Some(name), Some(value)) =
                            (caps.get(2), caps.get(3).or_else(|| caps.get(4)))
                        else {
                            continue;
                        };
                        let name = name.as_str().to_ascii_lowercase();
                        if name == "srcset" {
                            // Not checked by the media validator.
                            continue;
                        }
                        let decoded = html_escape::decode_html_entities(value.as_str());
                        let (construct, href) = if name == "href" {
                            (Construct::HtmlHref, transform_link(&decoded, config))
                        } else {
                            (Construct::HtmlSrc, transform_asset_url(&decoded, config))
                        };
                        let whole = caps.get(0).map_or(0..0, |m| m.range());
                        occurrences.push(Occurrence {
                            line: first_line + html[..whole.start].matches('\n').count(),
                            construct,
                            authored: excerpt(enclosing_tag(html, whole)),
                            destination: decoded.into_owned(),
                            wikilink_text: None,
                            key: href_key(&href),
                        });
                    }
                }
                _ => {}
            }
        }

        Self {
            occurrences,
            source,
            line_starts,
        }
    }

    /// The references whose emitted href is `href`, of the given constructs.
    fn matching<'a>(&'a self, href: &str, constructs: &'a [Construct]) -> Vec<&'a Occurrence> {
        let key = href_key(href);
        self.occurrences
            .iter()
            .filter(|occ| constructs.contains(&occ.construct) && occ.key == key)
            .collect()
    }

    /// Source lines outside fenced code that contain `needle`, with the
    /// line's text: the fallback for a reference no occurrence explains.
    /// With `include_fenced`, lines inside fences count too — a ```` ```chat ````
    /// fence is rendered, links and all.
    fn lines_containing(&self, needle: &str, include_fenced: bool) -> Vec<(usize, String)> {
        if needle.is_empty() {
            return Vec::new();
        }
        let mut fence: Option<(char, usize)> = None;
        let mut found = Vec::new();
        for (index, start) in self.line_starts.iter().enumerate() {
            let end = self
                .line_starts
                .get(index + 1)
                .copied()
                .unwrap_or(self.source.len());
            let line = &self.source[*start..end];
            let trimmed = line.trim_start();
            let run = |c: char| trimmed.chars().take_while(|&x| x == c).count();
            let opener = ['`', '~']
                .into_iter()
                .map(|c| (c, run(c)))
                .find(|(_, n)| *n >= 3);
            match (fence, opener) {
                (Some((c, n)), Some((oc, on))) if c == oc && on >= n => {
                    fence = None;
                    continue;
                }
                (None, Some(open)) => {
                    fence = Some(open);
                    continue;
                }
                (Some(_), _) if !include_fenced => continue,
                _ => {}
            }
            if line.contains(needle) {
                found.push((index + 1, excerpt(line)));
            }
        }
        found
    }
}

/// The tag of `html` around the attribute at `range` (`<img src="x.png">`),
/// else the attribute alone.
fn enclosing_tag(html: &str, range: std::ops::Range<usize>) -> &str {
    let start = html[..range.start].rfind('<');
    let end = html[range.end..].find('>').map(|at| range.end + at + 1);
    match (start, end) {
        (Some(start), Some(end)) if !html[start..range.start].contains('>') => &html[start..end],
        _ => html[range].trim(),
    }
}

/// `Text` in `[[Name|Text]]`.
fn wikilink_display(authored: &str) -> Option<String> {
    let inner = authored.strip_prefix("[[")?.strip_suffix("]]")?;
    inner.split_once('|').map(|(_, text)| text.to_string())
}

// ---------------------------------------------------------------------------
// Turning findings into located, suggested problems
// ---------------------------------------------------------------------------

struct NoteContext<'a> {
    checker: &'a LinkChecker,
    /// Repository-relative path of the note.
    source_path: &'a str,
    page_url: &'a str,
    located: &'a Located<'a>,
}

/// A problem before it is placed: everything but where.
struct Draft {
    kind: ProblemKind,
    reason: Reason,
    media: Option<MediaKind>,
    text: Option<String>,
    href: Option<String>,
    target: Option<String>,
    /// The decoded site path the matcher looks for.
    lookup: Option<String>,
    resolved_to: Option<String>,
    candidates: Vec<String>,
}

impl NoteContext<'_> {
    fn problems(&self, found: Found) -> Vec<Problem> {
        let link_constructs = [Construct::Link, Construct::Wikilink, Construct::HtmlHref];
        let media_constructs = [Construct::Image, Construct::HtmlSrc];

        let links = found.links.into_iter().flat_map(|broken| {
            let path_part = path_part(&broken.href);
            let checked = resolve_relative_url_checked(self.page_url, path_part, true);
            let resolved = checked
                .clone()
                .unwrap_or_else(|| resolve_relative_url(self.page_url, path_part, true));
            let target = decode(&resolved);
            let reason = match broken.defect {
                LinkDefect::Missing => Reason::Missing,
                LinkDefect::EscapesRoot => Reason::EscapesRoot,
                LinkDefect::NonCanonical => Reason::MissingTrailingSlash,
            };
            let draft = Draft {
                kind: ProblemKind::BrokenLink,
                reason,
                media: None,
                text: Some(broken.text).filter(|t| !t.is_empty()),
                href: Some(broken.href.clone()),
                target: checked.map(|_| target.clone()),
                lookup: Some(target),
                resolved_to: None,
                candidates: Vec::new(),
            };
            let occurrences = self.located.matching(&broken.href, &link_constructs);
            self.place(draft, occurrences, &broken.href)
        });

        let media = found.media.into_iter().flat_map(|(src, kind)| {
            let target = decode(&resolve_relative_url(self.page_url, path_part(&src), true));
            let target = target.trim_end_matches('/').to_string();
            let draft = Draft {
                kind: ProblemKind::BrokenMedia,
                reason: Reason::Missing,
                media: Some(kind),
                text: None,
                href: Some(src.clone()),
                target: Some(target.clone()),
                lookup: Some(target),
                resolved_to: None,
                candidates: Vec::new(),
            };
            let occurrences = self.located.matching(&src, &media_constructs);
            self.place(draft, occurrences, &src)
        });

        let unresolved = found.unresolved.into_iter().flat_map(|raw| {
            let raw = html_escape::decode_html_entities(&raw).into_owned();
            let name = raw
                .trim_start_matches("[[")
                .trim_end_matches("]]")
                .split(['|', '#'])
                .next()
                .unwrap_or_default()
                .trim()
                .to_string();
            let lines = self.located.lines_containing(&raw, false);
            let draft = |line: Option<usize>| Problem {
                line,
                kind: ProblemKind::UnresolvedWikilink,
                reason: Reason::Unresolved,
                syntax: Some(Syntax::Wikilink),
                media: None,
                authored: Some(raw.clone()),
                destination: Some(name.clone()),
                text: None,
                href: None,
                target: None,
                suggestions: Vec::new(),
                folder: None,
                resolved_to: None,
                candidates: Vec::new(),
            };
            let style = DestinationStyle::Wikilink {
                text: wikilink_display(&raw),
                suffix: String::new(),
            };
            let lookup = format!("/{name}");
            let problems: Vec<Problem> = if lines.is_empty() {
                vec![draft(None)]
            } else {
                lines
                    .into_iter()
                    .map(|(line, _)| draft(Some(line)))
                    .collect()
            };
            problems
                .into_iter()
                .map(|problem| self.suggest(problem, &lookup, &style))
                .collect::<Vec<_>>()
        });

        let ambiguous = found.ambiguous.into_iter().flat_map(|found| {
            let name = found
                .raw
                .trim_start_matches("[[")
                .trim_end_matches("]]")
                .to_string();
            let occurrences: Vec<&Occurrence> = self
                .located
                .occurrences
                .iter()
                .filter(|occ| {
                    occ.construct == Construct::Wikilink
                        && occ.destination.split('#').next().unwrap_or_default().trim() == name
                })
                .collect();
            let candidates: Vec<String> = std::iter::once(found.resolved_to.clone())
                .chain(found.candidates.iter().cloned())
                .collect();
            let draft = Draft {
                kind: ProblemKind::AmbiguousWikilink,
                reason: Reason::Ambiguous,
                media: None,
                text: None,
                href: None,
                target: None,
                lookup: None,
                resolved_to: Some(found.resolved_to.clone()),
                candidates,
            };
            let mut problems = self.place(draft, occurrences, "");
            for problem in &mut problems {
                if problem.authored.is_none() {
                    problem.authored = Some(found.raw.clone());
                }
            }
            problems
        });

        links
            .chain(media)
            .chain(unresolved)
            .chain(ambiguous)
            .collect()
    }

    /// One problem per occurrence (a broken href written on three lines is
    /// three todo items); without one, the lines naming the target's file,
    /// else a single problem with no line.
    fn place(&self, draft: Draft, occurrences: Vec<&Occurrence>, href: &str) -> Vec<Problem> {
        let base = Problem {
            line: None,
            kind: draft.kind,
            reason: draft.reason,
            syntax: None,
            media: draft.media,
            authored: None,
            destination: None,
            text: draft.text,
            href: draft.href,
            target: draft.target,
            suggestions: Vec::new(),
            folder: None,
            resolved_to: draft.resolved_to,
            candidates: Vec::new(),
        };
        let lookup = draft.lookup;
        let candidates = draft.candidates;

        let mut placed: Vec<(Problem, DestinationStyle)> = if !occurrences.is_empty() {
            occurrences
                .into_iter()
                .map(|occ| {
                    let problem = Problem {
                        line: Some(occ.line),
                        syntax: Some(occ.construct.syntax()),
                        authored: Some(occ.authored.clone()),
                        destination: Some(occ.destination.clone()),
                        ..base.clone()
                    };
                    (problem, DestinationStyle::of(occ))
                })
                .collect()
        } else {
            let file = file_name(path_part(&decode(href)).trim_end_matches('/')).to_string();
            let lines = if href.is_empty() {
                Vec::new()
            } else {
                Some(self.located.lines_containing(&file, false))
                    .filter(|lines| !lines.is_empty())
                    .unwrap_or_else(|| self.located.lines_containing(&file, true))
            };
            let fallback_style = || DestinationStyle::of_href(href);
            if lines.is_empty() {
                vec![(base, fallback_style())]
            } else {
                lines
                    .into_iter()
                    .map(|(line, text)| {
                        let problem = Problem {
                            line: Some(line),
                            authored: Some(text),
                            ..base.clone()
                        };
                        (problem, fallback_style())
                    })
                    .collect()
            }
        };

        placed
            .drain(..)
            .map(|(mut problem, style)| {
                if !candidates.is_empty() {
                    problem.candidates = self.candidates(&candidates, &style);
                    return problem;
                }
                match (&lookup, problem.reason) {
                    (Some(_), Reason::MissingTrailingSlash) => {
                        problem.suggestions = self.add_slash(&problem);
                        problem
                    }
                    (Some(lookup), _) => self.suggest(problem, lookup, &style),
                    (None, _) => problem,
                }
            })
            .collect()
    }

    /// "Did you mean" for a missing target, else the surviving folder.
    fn suggest(&self, mut problem: Problem, lookup: &str, style: &DestinationStyle) -> Problem {
        let index = &self.checker.suggestions;
        problem.suggestions = index
            .did_you_mean(lookup, true, MAX_DID_YOU_MEAN)
            .iter()
            .map(|page| self.checker.suggestion(page, style, self.source_path))
            .collect();
        // A wikilink names a note, not a place: its "folder" is only the
        // note's own, which says nothing.
        let names_a_place = !matches!(style, DestinationStyle::Wikilink { .. });
        if names_a_place && problem.suggestions.is_empty() && !index.page_exists(lookup) {
            problem.folder = index.folder_fallback(lookup).map(|folder| FolderHint {
                url: folder.url,
                title: folder.title,
            });
        }
        problem
    }

    /// A page link missing its trailing slash: the same destination with one.
    fn add_slash(&self, problem: &Problem) -> Vec<Suggestion> {
        let Some(destination) = &problem.destination else {
            return Vec::new();
        };
        let cut = destination.find(['#', '?']).unwrap_or(destination.len());
        let (path, suffix) = destination.split_at(cut);
        if path.is_empty() || path.ends_with('/') {
            return Vec::new();
        }
        vec![Suggestion {
            replacement: format!("{path}/{suffix}"),
            url: problem.target.clone().unwrap_or_default(),
            path: None,
            title: None,
        }]
    }

    /// An ambiguous wikilink's candidates, each written so it names one note.
    fn candidates(&self, urls: &[String], style: &DestinationStyle) -> Vec<Suggestion> {
        let index = &self.checker.suggestions;
        urls.iter()
            .map(|url| {
                let page = index.note_at(url).unwrap_or_else(|| SuggestedPage {
                    url: url.clone(),
                    title: url.clone(),
                    kind: SuggestionKind::Note,
                    index: usize::MAX,
                });
                let mut suggestion = self.checker.suggestion(&page, style, self.source_path);
                if page.index == usize::MAX {
                    suggestion.title = None;
                }
                // Every candidate shares the name: always the path form.
                suggestion.replacement = style.write(&Target {
                    url: &page.url,
                    path: suggestion.path.as_deref(),
                    kind: SuggestionKind::Note,
                    is_index: suggestion
                        .path
                        .as_deref()
                        .is_some_and(|p| file_name(p) == self.checker.index_file),
                    unique_name: false,
                    source_path: self.source_path,
                });
                suggestion
            })
            .collect()
    }
}

/// An href without its fragment and query.
fn path_part(href: &str) -> &str {
    href.split(['#', '?']).next().unwrap_or_default()
}

fn decode(url: &str) -> String {
    percent_encoding::percent_decode_str(url)
        .decode_utf8_lossy()
        .into_owned()
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name,
    }
}

// ---------------------------------------------------------------------------
// Writing a replacement in the author's style
// ---------------------------------------------------------------------------

/// How the author wrote the broken destination, which is how its
/// replacement is written.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DestinationStyle {
    /// `[[Name]]` (with `|Text` kept): the note's name when it is unique,
    /// else its path from the source's folder.
    Wikilink {
        text: Option<String>,
        suffix: String,
    },
    /// `/docs/guide/` or, with `extension`, `/docs/guide.md`.
    RootRelative { extension: bool, suffix: String },
    /// `../docs/guide.md`, or without `extension` `../docs/guide` (`/`
    /// appended with `slash`), relative to the source file's folder.
    Relative {
        extension: bool,
        slash: bool,
        suffix: String,
        /// Spaces written `%20` (a markdown destination; a raw HTML attribute
        /// keeps them).
        encode_spaces: bool,
    },
}

/// The fragment and query of a destination (`#setup`, `?a=1#x`).
fn suffix_of(destination: &str) -> String {
    destination
        .find(['#', '?'])
        .map_or_else(String::new, |at| destination[at..].to_string())
}

fn has_markdown_extension(path: &str) -> bool {
    let name = file_name(path);
    name.rfind('.').is_some_and(|dot| {
        let ext = name[dot + 1..].to_ascii_lowercase();
        [
            "md", "markdown", "mdown", "mkd", "mkdn", "mdwn", "mdx", "txt",
        ]
        .contains(&ext.as_str())
    })
}

impl DestinationStyle {
    fn of(occurrence: &Occurrence) -> Self {
        let destination = occurrence.destination.as_str();
        match occurrence.construct {
            Construct::Wikilink => Self::Wikilink {
                text: occurrence.wikilink_text.clone(),
                suffix: destination
                    .find('#')
                    .map_or_else(String::new, |at| destination[at..].to_string()),
            },
            construct => Self::of_destination(
                destination,
                matches!(construct, Construct::Link | Construct::Image),
            ),
        }
    }

    fn of_destination(destination: &str, markdown: bool) -> Self {
        let suffix = suffix_of(destination);
        let path = path_part(destination);
        if path.starts_with('/') {
            Self::RootRelative {
                extension: has_markdown_extension(path),
                suffix,
            }
        } else {
            Self::Relative {
                extension: has_markdown_extension(path) || !path.ends_with('/'),
                slash: path.ends_with('/'),
                suffix,
                encode_spaces: markdown,
            }
        }
    }

    /// The style for a reference located only by its emitted href: a
    /// root-relative URL, which reads the same from anywhere.
    fn of_href(href: &str) -> Self {
        Self::RootRelative {
            extension: false,
            suffix: suffix_of(href),
        }
    }

    fn write(&self, target: &Target<'_>) -> String {
        match self {
            Self::Wikilink { text, suffix } => {
                let name = match (target.kind, target.path) {
                    (SuggestionKind::Note, Some(path))
                        if target.unique_name && !target.is_index =>
                    {
                        stem(file_name(path)).to_string()
                    }
                    (SuggestionKind::Note, Some(path)) if target.is_index => {
                        let folder = relative_path(target.source_path, folder_of(path));
                        format!("{}/", folder.trim_end_matches('/'))
                    }
                    (SuggestionKind::Note, Some(path)) => {
                        let relative = relative_path(target.source_path, path);
                        let relative = strip_markdown_extension(&relative);
                        if relative.contains('/') {
                            relative.to_string()
                        } else {
                            format!("./{relative}")
                        }
                    }
                    (_, Some(path)) => relative_path(target.source_path, path),
                    (_, None) => target.url.to_string(),
                };
                // A label that repeats the link exactly is what `[[name]]`
                // already displays, so it is dropped. With a `#fragment`
                // the default display differs, so the label stays.
                let text = match text.as_deref() {
                    Some(text) if !(suffix.is_empty() && text == name) => format!("|{text}"),
                    _ => String::new(),
                };
                format!("[[{name}{suffix}{text}]]")
            }
            Self::RootRelative { extension, suffix } => {
                let base = match (target.path, extension) {
                    (Some(path), true) if target.kind == SuggestionKind::Note => format!("/{path}"),
                    _ => target.url.to_string(),
                };
                format!("{}{suffix}", base.replace(' ', "%20"))
            }
            Self::Relative {
                extension,
                slash,
                suffix,
                encode_spaces,
            } => {
                let base = match (target.kind, target.path) {
                    (SuggestionKind::Note, Some(path)) if *extension => {
                        relative_path(target.source_path, path)
                    }
                    (SuggestionKind::Note, Some(path)) if target.is_index => {
                        let folder = relative_path(target.source_path, folder_of(path));
                        format!("{}/", folder.trim_end_matches('/'))
                    }
                    (SuggestionKind::Note, Some(path)) => {
                        let relative = relative_path(target.source_path, path);
                        let relative = strip_markdown_extension(&relative).to_string();
                        if *slash {
                            format!("{relative}/")
                        } else {
                            relative
                        }
                    }
                    (SuggestionKind::File, Some(path)) => relative_path(target.source_path, path),
                    (_, None) => target.url.to_string(),
                };
                let base = if *encode_spaces {
                    base.replace(' ', "%20")
                } else {
                    base
                };
                format!("{base}{suffix}")
            }
        }
    }
}

/// What a replacement points at.
struct Target<'a> {
    url: &'a str,
    /// Repository-relative path, when in the repository.
    path: Option<&'a str>,
    kind: SuggestionKind,
    is_index: bool,
    /// No other note answers to this note's file stem.
    unique_name: bool,
    /// Repository-relative path of the note the reference is in.
    source_path: &'a str,
}

fn folder_of(path: &str) -> &str {
    path.rfind('/').map_or("", |at| &path[..at])
}

fn strip_markdown_extension(path: &str) -> &str {
    if has_markdown_extension(path) {
        path.rfind('.').map_or(path, |dot| &path[..dot])
    } else {
        path
    }
}

/// `to` (repository-relative) as a path relative to the folder of
/// `from_file` (repository-relative): the way a markdown destination is
/// written. `to` may be a folder (`""` for the root), which yields `./` when
/// it is the source's own folder.
fn relative_path(from_file: &str, to: &str) -> String {
    let from_dir: Vec<&str> = folder_of(from_file)
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let to_segs: Vec<&str> = to.split('/').filter(|s| !s.is_empty()).collect();
    let common = from_dir
        .iter()
        .zip(&to_segs)
        .take_while(|(a, b)| a == b)
        .count();
    let up = "../".repeat(from_dir.len() - common);
    let rest = to_segs[common..].join("/");
    match (up.is_empty(), rest.is_empty()) {
        (true, true) => "./".to_string(),
        (false, true) => up,
        _ => format!("{up}{rest}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// A repository on disk from `(path, contents)` pairs.
    fn repo(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().expect("tempdir");
        fs::create_dir_all(dir.path().join(".mbr")).expect("mkdir .mbr");
        for (path, contents) in files {
            let full = dir.path().join(path);
            fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
            fs::write(full, contents).expect("write");
        }
        dir
    }

    fn check(dir: &TempDir) -> LinkReport {
        let config = Config {
            root_dir: dir.path().canonicalize().expect("canonical root"),
            ..Config::default()
        };
        LinkChecker::scan(&config).expect("scan").check_all(Some(2))
    }

    fn problems<'r>(report: &'r LinkReport, path: &str) -> &'r [Problem] {
        report
            .files
            .iter()
            .find(|file| file.path == path)
            .map_or(&[], |file| file.problems.as_slice())
    }

    fn replacements(problem: &Problem) -> Vec<&str> {
        problem
            .suggestions
            .iter()
            .map(|s| s.replacement.as_str())
            .collect()
    }

    // ---- paths and styles ----

    #[test]
    fn relative_path_climbs_to_the_common_folder() {
        assert_eq!(
            relative_path("docs/guide.md", "install/setup.md"),
            "../install/setup.md"
        );
        assert_eq!(relative_path("docs/guide.md", "docs/other.md"), "other.md");
        assert_eq!(relative_path("guide.md", "a/b/c.md"), "a/b/c.md");
        assert_eq!(relative_path("a/b/c.md", "x.md"), "../../x.md");
        assert_eq!(relative_path("docs/guide.md", "docs"), "./");
        assert_eq!(relative_path("docs/guide.md", ""), "../");
    }

    fn target<'a>(path: &'a str, url: &'a str, source: &'a str) -> Target<'a> {
        Target {
            url,
            path: Some(path),
            kind: SuggestionKind::Note,
            is_index: file_name(path) == "index.md",
            unique_name: true,
            source_path: source,
        }
    }

    #[test]
    fn style_keeps_a_relative_markdown_path_and_its_fragment() {
        let style = DestinationStyle::of_destination("../setup.md#install", true);
        let t = target("install/setup.md", "/install/setup/", "docs/guide.md");
        assert_eq!(style.write(&t), "../install/setup.md#install");
    }

    #[test]
    fn style_keeps_an_extensionless_relative_path_and_trailing_slash() {
        let t = target("install/setup.md", "/install/setup/", "docs/guide.md");
        assert_eq!(
            DestinationStyle::of_destination("../setup/", true).write(&t),
            "../install/setup/"
        );
        let index = target("install/index.md", "/install/", "docs/guide.md");
        assert_eq!(
            DestinationStyle::of_destination("../setup/", true).write(&index),
            "../install/"
        );
    }

    #[test]
    fn style_keeps_root_relative_urls_root_relative() {
        let t = target("notes/api.md", "/notes/api/", "docs/guide.md");
        assert_eq!(
            DestinationStyle::of_destination("/docs/api/", true).write(&t),
            "/notes/api/"
        );
        assert_eq!(
            DestinationStyle::of_destination("/docs/api.md", true).write(&t),
            "/notes/api.md"
        );
    }

    #[test]
    fn style_writes_a_wikilink_by_name_or_by_path_when_the_name_is_shared() {
        let style = DestinationStyle::Wikilink {
            text: Some("shown".to_string()),
            suffix: "#part".to_string(),
        };
        let unique = target("notes/New Name.md", "/notes/New Name/", "todo.md");
        assert_eq!(style.write(&unique), "[[New Name#part|shown]]");
        let shared = Target {
            unique_name: false,
            ..target(
                "people/a/John Doe.md",
                "/people/a/John Doe/",
                "notes/todo.md",
            )
        };
        assert_eq!(style.write(&shared), "[[../people/a/John Doe#part|shown]]");
    }

    #[test]
    fn style_drops_a_wikilink_label_only_when_it_repeats_the_name() {
        let labelled = |text: &str, suffix: &str| DestinationStyle::Wikilink {
            text: Some(text.to_string()),
            suffix: suffix.to_string(),
        };
        let unique = target(
            "nclh/NCLH Architecture Notes.md",
            "/nclh/NCLH Architecture Notes/",
            "todo.md",
        );
        assert_eq!(
            labelled("NCLH Architecture Notes", "").write(&unique),
            "[[NCLH Architecture Notes]]"
        );
        // A different label, even only in case, is kept.
        assert_eq!(
            labelled("Notes", "").write(&unique),
            "[[NCLH Architecture Notes|Notes]]"
        );
        assert_eq!(
            labelled("nclh architecture notes", "").write(&unique),
            "[[NCLH Architecture Notes|nclh architecture notes]]"
        );
        // With a fragment the default display differs, so the label stays.
        assert_eq!(
            labelled("NCLH Architecture Notes", "#part").write(&unique),
            "[[NCLH Architecture Notes#part|NCLH Architecture Notes]]"
        );
        // Written by path, the stem as label is not the name.
        let shared = Target {
            unique_name: false,
            ..target(
                "people/a/John Doe.md",
                "/people/a/John Doe/",
                "notes/todo.md",
            )
        };
        assert_eq!(
            labelled("John Doe", "").write(&shared),
            "[[../people/a/John Doe|John Doe]]"
        );
    }

    #[test]
    fn style_writes_media_as_a_relative_file_path_with_encoded_spaces() {
        let file = Target {
            kind: SuggestionKind::File,
            is_index: false,
            unique_name: false,
            ..target("assets/my flow.png", "/assets/my flow.png", "docs/guide.md")
        };
        assert_eq!(
            DestinationStyle::of_destination("img/flow.png", true).write(&file),
            "../assets/my%20flow.png"
        );
        // A raw HTML attribute keeps the space.
        assert_eq!(
            DestinationStyle::of_destination("img/flow.png", false).write(&file),
            "../assets/my flow.png"
        );
    }

    #[test]
    fn inline_code_survives_backticks() {
        assert_eq!(code("a"), "`a`");
        assert_eq!(code("a`b"), "`` a`b ``");
    }

    // ---- line mapping ----

    #[test]
    fn scan_records_lines_counting_frontmatter_and_skipping_code() {
        let source = "---\ntitle: T\n---\n# T\n\n[a](x.md) and ![i](y.png)\n\n```\n[no](z.md)\n```\n\n<a href=\"w.md\">w</a>\n\n- [[Name|shown]]\n";
        let located = Located::scan(
            source,
            &LinkTransformConfig::default(),
            &HashSet::new(),
            None,
        );
        let seen: Vec<(usize, Construct, &str)> = located
            .occurrences
            .iter()
            .map(|o| (o.line, o.construct, o.destination.as_str()))
            .collect();
        assert_eq!(
            seen,
            [
                (6, Construct::Link, "x.md"),
                (6, Construct::Image, "y.png"),
                (12, Construct::HtmlHref, "w.md"),
                (14, Construct::Wikilink, "Name"),
            ]
        );
        assert_eq!(
            located.occurrences[3].wikilink_text.as_deref(),
            Some("shown")
        );
        assert_eq!(located.occurrences[2].authored, "<a href=\"w.md\">");
        // The key is what the renderer emits (`../x/` from a non-index page).
        assert_eq!(located.occurrences[0].key, "../x/");
    }

    #[test]
    fn lines_containing_skips_fenced_code() {
        let source = "a.png\n```\na.png\n```\n~~~~\n```\na.png\n~~~~\nsee a.png\n";
        let located = Located::scan(
            source,
            &LinkTransformConfig::default(),
            &HashSet::new(),
            None,
        );
        let lines: Vec<usize> = located
            .lines_containing("a.png", false)
            .into_iter()
            .map(|(line, _)| line)
            .collect();
        assert_eq!(lines, [1, 9]);
        let everywhere: Vec<usize> = located
            .lines_containing("a.png", true)
            .into_iter()
            .map(|(line, _)| line)
            .collect();
        assert_eq!(everywhere, [1, 3, 7, 9]);
    }

    // ---- whole repositories ----

    #[test]
    fn clean_repository_has_no_problems() {
        let dir = repo(&[
            ("index.md", "# Home\n\n[Guide](docs/guide.md)\n"),
            (
                "docs/guide.md",
                "# Guide\n\n[Home](../index.md) ![x](pic.png)\n",
            ),
            ("docs/pic.png", "png"),
        ]);
        let report = check(&dir);
        assert!(!report.has_problems(), "{report:#?}");
        assert_eq!(report.summary.files_checked, 2);
        assert!(
            report
                .to_markdown()
                .starts_with("No broken links in 2 files.")
        );
    }

    #[test]
    fn moved_note_is_found_by_its_file_name_first() {
        let dir = repo(&[
            (
                "docs/guide.md",
                "---\ntitle: Guide\n---\n# Guide\n\nSee [Setup](../setup.md#run).\n",
            ),
            ("install/setup.md", "# Setup\n"),
            ("install/setup-advanced.md", "# Advanced\n"),
            ("setup-old/notes.md", "# Notes\n"),
        ]);
        let report = check(&dir);
        let found = problems(&report, "docs/guide.md");
        assert_eq!(found.len(), 1, "{found:#?}");
        let problem = &found[0];
        assert_eq!(problem.line, Some(6));
        assert_eq!(problem.kind, ProblemKind::BrokenLink);
        assert_eq!(problem.reason, Reason::Missing);
        assert_eq!(problem.syntax, Some(Syntax::Link));
        assert_eq!(
            problem.authored.as_deref(),
            Some("[Setup](../setup.md#run)")
        );
        assert_eq!(problem.target.as_deref(), Some("/setup/"));
        assert_eq!(
            replacements(problem),
            [
                "../install/setup.md#run",
                "../install/setup-advanced.md#run"
            ]
        );
        assert_eq!(problem.suggestions[0].url, "/install/setup/");
        assert_eq!(
            problem.suggestions[0].path.as_deref(),
            Some("install/setup.md")
        );
    }

    #[test]
    fn every_occurrence_of_one_broken_href_is_its_own_item() {
        let dir = repo(&[
            ("a.md", "# A\n\n[x](gone.md)\n\ntext\n\n[again](gone.md)\n"),
            ("b/gone.md", "# Gone\n"),
        ]);
        let report = check(&dir);
        let lines: Vec<Option<usize>> = problems(&report, "a.md").iter().map(|p| p.line).collect();
        assert_eq!(lines, [Some(3), Some(7)]);
    }

    #[test]
    fn broken_media_suggests_a_relative_file_path() {
        let dir = repo(&[
            (
                "docs/guide.md",
                "# Guide\n\n![diagram](img/flow.png)\n\n<img src=\"img/flow.png\">\n",
            ),
            ("assets/flow.png", "png"),
        ]);
        let report = check(&dir);
        let found = problems(&report, "docs/guide.md");
        assert_eq!(found.len(), 2, "{found:#?}");
        for problem in found {
            assert_eq!(problem.kind, ProblemKind::BrokenMedia);
            assert_eq!(problem.media, Some(MediaKind::Image));
            assert_eq!(replacements(problem), ["../assets/flow.png"]);
        }
        assert_eq!(found[0].syntax, Some(Syntax::Image));
        assert_eq!(found[1].syntax, Some(Syntax::Html));
        assert_eq!(found[1].line, Some(5));
    }

    /// Regression: an extension the matcher's extension rule does not read
    /// (`x-m4a`, a dash) still names a file, so a moved one is found.
    #[test]
    fn moved_file_with_a_dash_in_its_extension_is_suggested() {
        let dir = repo(&[
            (
                "a/note.md",
                "# Note\n\n![][ref]\n\n[ref]: 2016-12-13--ee.x-m4a\n",
            ),
            ("a/other.md", "# Other\n"),
            ("b/2016-12-13--ee.x-m4a", "audio"),
        ]);
        let report = check(&dir);
        let found = problems(&report, "a/note.md");
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(replacements(&found[0]), ["../b/2016-12-13--ee.x-m4a"]);
    }

    #[test]
    fn root_relative_link_gets_a_root_relative_suggestion() {
        let dir = repo(&[
            ("a.md", "# A\n\n[api](/docs/api/)\n"),
            ("notes/api.md", "# API\n"),
        ]);
        let report = check(&dir);
        let found = problems(&report, "a.md");
        assert_eq!(replacements(&found[0]), ["/notes/api/"]);
    }

    #[test]
    fn unresolved_and_ambiguous_wikilinks() {
        let dir = repo(&[
            (
                "notes/todo.md",
                "# Todo\n\n- [[New Nam|the new one]]\n- [[John Doe]]\n",
            ),
            ("notes/New Name.md", "# New\n"),
            ("people/a/John Doe.md", "# J\n"),
            ("people/b/John Doe.md", "# J\n"),
        ]);
        let report = check(&dir);
        let found = problems(&report, "notes/todo.md");
        assert_eq!(found.len(), 2, "{found:#?}");

        assert_eq!(found[0].line, Some(3));
        assert_eq!(found[0].syntax, Some(Syntax::Wikilink));
        assert_eq!(replacements(&found[0]), ["[[New Name|the new one]]"]);
        assert!(found[0].folder.is_none(), "no folder hint for a wikilink");

        let ambiguous = &found[1];
        assert_eq!(ambiguous.kind, ProblemKind::AmbiguousWikilink);
        assert_eq!(ambiguous.line, Some(4));
        assert_eq!(
            ambiguous.resolved_to.as_deref(),
            Some("/people/a/John Doe/")
        );
        let candidates: Vec<&str> = ambiguous
            .candidates
            .iter()
            .map(|c| c.replacement.as_str())
            .collect();
        assert_eq!(
            candidates,
            ["[[../people/a/John Doe]]", "[[../people/b/John Doe]]"]
        );
    }

    #[test]
    fn raw_html_escape_and_missing_slash_are_reported_by_defect() {
        let dir = repo(&[
            (
                "docs/a.md",
                "# A\n\n<a href=../../../../x.md>up</a>\n\n<a href=/install/setup>slash</a>\n",
            ),
            ("install/setup.md", "# Setup\n"),
        ]);
        let report = check(&dir);
        let found = problems(&report, "docs/a.md");
        let reasons: Vec<Reason> = found.iter().map(|p| p.reason).collect();
        assert_eq!(
            reasons,
            [Reason::EscapesRoot, Reason::MissingTrailingSlash],
            "{found:#?}"
        );
        // Unquoted values are not rewritten, so these are located by name.
        assert_eq!(found[0].line, Some(3));
        assert_eq!(found[1].line, Some(5));
    }

    #[test]
    fn a_link_in_a_chat_bubble_is_located_by_name() {
        let dir = repo(&[("a.md", "# A\n\n```chat\n{{Ann|see [[Gone Note]]|}}\n```\n")]);
        let report = check(&dir);
        let found = problems(&report, "a.md");
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].line, Some(4));
        assert_eq!(found[0].syntax, None);
    }

    #[test]
    fn folder_hint_when_nothing_matches() {
        let dir = repo(&[
            ("a.md", "# A\n\n[x](docs/zzzz-unmatched.md)\n"),
            ("docs/index.md", "---\ntitle: Docs\n---\n# Docs\n"),
        ]);
        let report = check(&dir);
        let problem = &problems(&report, "a.md")[0];
        assert!(problem.suggestions.is_empty());
        assert_eq!(
            problem.folder,
            Some(FolderHint {
                url: "/docs/".to_string(),
                title: "Docs".to_string()
            })
        );
        assert!(
            report
                .to_markdown()
                .contains("folder `/docs/` still exists")
        );
    }

    #[test]
    fn applying_every_suggestion_leaves_a_clean_report() {
        let files = [
            (
                "docs/guide.md",
                "# Guide\n\n[Setup](../setup.md) [api](/docs/api.md) ![d](img/flow.png)\n",
            ),
            ("notes/todo.md", "# Todo\n\n- [[New Nam]]\n"),
            ("install/setup.md", "# Setup\n"),
            ("notes/api.md", "# API\n"),
            ("notes/New Name.md", "# New\n"),
            ("assets/flow.png", "png"),
        ];
        let dir = repo(&files);
        let report = check(&dir);
        assert_eq!(report.summary.problems, 4, "{report:#?}");
        for file in &report.files {
            let full = dir.path().join(&file.path);
            let mut text = fs::read_to_string(&full).expect("read");
            for problem in &file.problems {
                let (Some(destination), Some(first)) =
                    (&problem.destination, problem.suggestions.first())
                else {
                    panic!("no suggestion: {problem:#?}");
                };
                let old = match problem.syntax {
                    Some(Syntax::Wikilink) => format!("[[{destination}]]"),
                    _ => destination.clone(),
                };
                text = text.replacen(&old, &first.replacement, 1);
            }
            fs::write(&full, text).expect("write");
        }
        let after = check(&dir);
        assert!(!after.has_problems(), "{}", after.to_markdown());
    }

    #[test]
    fn markdown_and_json_reports_name_every_problem() {
        let dir = repo(&[("a.md", "# A\n\n[x](gone.md)\n")]);
        let report = check(&dir);
        let markdown = report.to_markdown();
        assert!(
            markdown.starts_with("# Broken links: 1 in 1 file\n"),
            "{markdown}"
        );
        assert!(markdown.contains("## a.md\n\n- [ ] L3 `[x](gone.md)` — no such page or file\n"));
        let json: serde_json::Value = serde_json::from_str(&report.to_json()).expect("json");
        assert_eq!(json["schema_version"], REPORT_SCHEMA_VERSION);
        assert_eq!(json["files"][0]["problems"][0]["kind"], "broken_link");
        assert_eq!(json["files"][0]["problems"][0]["line"], 3);
    }

    #[test]
    fn unresolved_literal_wikilink_is_located_and_suggested() {
        let dir = repo(&[
            ("a.md", "# A\n\nsee [[Old Name]] here\n"),
            ("b/Old Name.md", "# Old\n"),
        ]);
        let config = Config {
            root_dir: dir.path().canonicalize().expect("root"),
            ..Config::default()
        };
        let checker = LinkChecker::scan(&config).expect("scan");
        let source = "# A\n\nsee [[Old Name]] here\n";
        let located = Located::scan(
            source,
            &LinkTransformConfig::default(),
            &HashSet::new(),
            None,
        );
        let context = NoteContext {
            checker: &checker,
            source_path: "a.md",
            page_url: "/a/",
            located: &located,
        };
        let found = Found {
            links: Vec::new(),
            media: Vec::new(),
            unresolved: vec!["[[Old Name]]".to_string()],
            ambiguous: Vec::new(),
        };
        let problems = context.problems(found);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].kind, ProblemKind::UnresolvedWikilink);
        assert_eq!(problems[0].line, Some(3));
        assert_eq!(replacements(&problems[0]), ["[[Old Name]]"]);
    }

    #[test]
    fn unreadable_note_is_reported_apart() {
        let dir = repo(&[("a.md", "# A\n"), ("b.md", "# B\n")]);
        let config = Config {
            root_dir: dir.path().canonicalize().expect("root"),
            ..Config::default()
        };
        let checker = LinkChecker::scan(&config).expect("scan");
        fs::remove_file(dir.path().join("b.md")).expect("remove");
        let report = checker.check_all(None);
        assert_eq!(report.summary.unreadable, 1);
        assert_eq!(report.summary.files_checked, 1);
        assert_eq!(report.unreadable[0].path, "b.md");
        assert!(!report.has_problems());
        assert!(report.to_markdown().contains("## Could not be checked"));
    }
}
