//! One note rendered to a whole HTML page with no server and no repository
//! scan: what `mbr -o` prints, what `mbr --pdf` checks a note's page template
//! with before it starts an export ([`check_page_template`]), and what an
//! embedding app renders a live preview of unsaved text with
//! ([`Renderer::render_source`], and [`Renderer::render_source_with_template`]
//! for unsaved template text).
//!
//! The page is the one the server would send, minus what needs a server: no
//! siblings, no backlinks, absolute URLs, and wikilinks resolved only against
//! an index the caller supplies ([`Renderer::with_wikilink_index`]). A note
//! with its own page template (`template:`) is rendered through it, resolved
//! inside the repository exactly as the server and the static build resolve
//! it ([`crate::page_template::resolve_page_template`]); a problem is an
//! error, never a silent fallback to the normal page.
//!
//! [`render_note`] and [`render_note_source`] are one-shot conveniences. A
//! caller that renders repeatedly (a preview re-rendered on every keystroke)
//! keeps one [`Renderer`]: it loads the Tera templates once, which is most of
//! a cold render's cost, and never scans the repository.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::Config;
use crate::errors::MbrError;
use crate::link_transform::LinkTransformConfig;
use crate::markdown::{self, MarkdownRenderResult};
use crate::oembed_cache::OembedCache;
use crate::page_context::{self, CalendarChrome, UrlMode};
use crate::page_template::{self, PageTemplateError};
use crate::relationships::RelationTypeRegistry;
use crate::templates::Templates;
use crate::vault::{LocalVault, Vault, VaultPath};
use crate::wikilink_index::WikilinkIndex;

/// Renders a repository's notes to whole pages, standalone, reusing
/// everything that does not depend on the note: the loaded templates, the
/// repository's vault and the settings derived from its configuration.
///
/// Building one costs a template load (`.mbr/` or the defaults); each render
/// then costs the markdown render, a `stat` of the note, and — for a note
/// with `template:` — reading and parsing that template and its sibling
/// `.html` files, which are read from disk on every render so an edit to
/// them shows at once. Mbr's own templates are not re-read: call
/// [`Self::reload_templates`] after a `.mbr/` change.
///
/// `Send + Sync`: one renderer can serve several threads.
pub struct Renderer {
    config: Config,
    templates: Templates,
    /// The repository root, canonicalized, for resolving page templates.
    vault: LocalVault,
    exempt_hidden_dirs: Vec<VaultPath>,
    valid_tag_sources: HashSet<String>,
    relation_registry: RelationTypeRegistry,
    calendar: CalendarChrome,
    /// Written by [`Self::render_file`]'s OpenGraph fetches, read by
    /// [`Self::render_source`], which never fetches.
    oembed_cache: Arc<OembedCache>,
    wikilink_index: Option<Arc<WikilinkIndex>>,
}

impl Renderer {
    /// A renderer for the repository at `config.root_dir`, with its
    /// templates (`template_folder`, else `.mbr/`, else the defaults).
    ///
    /// # Errors
    ///
    /// [`MbrError::Template`] when the templates cannot be loaded.
    pub fn new(config: &Config) -> Result<Self, MbrError> {
        let templates = Templates::new(&config.root_dir, config.template_folder.as_deref())
            .inspect_err(|e| tracing::error!("Error parsing template: {e}"))?;
        Ok(Self {
            templates,
            vault: LocalVault::new(&config.root_dir),
            exempt_hidden_dirs: config
                .explicit_hidden_dirs
                .iter()
                .filter_map(|dir| VaultPath::from_relative_native(dir).ok())
                .collect(),
            valid_tag_sources: crate::config::tag_sources_to_set(&config.tag_sources),
            relation_registry: RelationTypeRegistry::from_types(&config.relationship_types),
            calendar: CalendarChrome::new(&config.calendar),
            oembed_cache: Arc::new(OembedCache::new(config.oembed_cache_size)),
            wikilink_index: None,
            config: config.clone(),
        })
    }

    /// Resolves body wikilinks (`[[Acme]]`) against `index` — typically a
    /// running server's (`server.repo().wikilink_index`), which stays current
    /// as the repository changes — instead of leaving them unresolved.
    #[must_use]
    pub fn with_wikilink_index(mut self, index: Arc<WikilinkIndex>) -> Self {
        self.wikilink_index = Some(index);
        self
    }

    /// Re-reads the templates from disk (after a `.mbr/` change).
    ///
    /// # Errors
    ///
    /// [`MbrError::Template`] when they cannot be loaded; the previous set
    /// stays in use.
    pub fn reload_templates(&self) -> Result<(), MbrError> {
        Ok(self.templates.reload()?)
    }

    /// Renders `source` as the note at `relative_path` (from the repository
    /// root) to a whole page: the page [`Self::render_file`] gives for that
    /// file holding `source`, through the note's page template when its
    /// frontmatter names one, with links resolved relative to
    /// `relative_path`. The file need not exist (a note not saved yet).
    ///
    /// Synchronous and never on the network: a bare URL gets an OpenGraph
    /// card only when this renderer's [`Self::render_file`] already fetched
    /// it. The YouTube, Giphy and media embeds need no network and are
    /// always produced.
    ///
    /// # Errors
    ///
    /// [`MbrError::PageTemplate`] for a page template that is missing,
    /// refused or fails to render; [`MbrError::Template`] when `index.html`
    /// fails.
    pub fn render_source(&self, relative_path: &Path, source: &str) -> Result<String, MbrError> {
        self.render_source_through(relative_path, source, PageTemplateChoice::Note)
    }

    /// [`Self::render_source`] through **unsaved** template text: `source`
    /// as the note at `relative_path`, rendered through `template_source` as
    /// the page template file at `template_path` (from the repository root; a
    /// leading `/`, as `template:` writes it, is allowed). For a template
    /// editor's live preview, re-rendered on every edit.
    ///
    /// The note's own `template:` is not consulted: the page is always
    /// rendered through the given template. Its path follows the rules of a
    /// `template:` value (inside the repository, not hidden, `.html`) but the
    /// file need not exist yet. The `.html` files beside it (what it includes
    /// or extends) and mbr's templates come from disk, as for any render; its
    /// own saved file does not. Nothing is cached: the next
    /// [`Self::render_source`] reads the saved template again, so a broken
    /// draft affects only this call.
    ///
    /// # Errors
    ///
    /// [`MbrError::PageTemplate`] when the path is refused, the text is
    /// larger than [`page_template::MAX_TEMPLATE_BYTES`], or the text does
    /// not parse or render — [`PageTemplateError::Render`], naming the
    /// template and carrying Tera's message (line and column of a syntax
    /// error).
    pub fn render_source_with_template(
        &self,
        relative_path: &Path,
        source: &str,
        template_path: &Path,
        template_source: &str,
    ) -> Result<String, MbrError> {
        self.render_source_through(
            relative_path,
            source,
            PageTemplateChoice::Unsaved {
                path: template_path,
                source: template_source,
            },
        )
    }

    /// `source` as the note at `relative_path`, rendered with no I/O but the
    /// page template's, through the page template `choice` names.
    fn render_source_through(
        &self,
        relative_path: &Path,
        source: &str,
        choice: PageTemplateChoice<'_>,
    ) -> Result<String, MbrError> {
        let absolute_path = self.config.root_dir.join(relative_path);
        let render_result =
            markdown::render_str(source, &absolute_path, self.render_options(&absolute_path))
                .inspect_err(|e| tracing::error!("Error rendering markdown: {:?}", e))?;
        self.page(&absolute_path, relative_path, render_result, choice)
    }

    /// Renders the markdown file at `absolute_path` (`relative_path` from the
    /// repository root) to a whole page, through its page template when it
    /// names one. Bare URLs are enriched over the network when
    /// `oembed_timeout_ms` is above zero.
    ///
    /// # Errors
    ///
    /// As [`Self::render_source`], plus [`MbrError::Markdown`] when the file
    /// cannot be read.
    pub async fn render_file(
        &self,
        absolute_path: &Path,
        relative_path: &Path,
    ) -> Result<String, MbrError> {
        let source = tokio::fs::read_to_string(absolute_path)
            .await
            .map_err(|e| crate::errors::MarkdownError::ReadFailed {
                path: absolute_path.to_path_buf(),
                source: e,
            })
            .inspect_err(|e| tracing::error!("Error rendering markdown: {:?}", e))?;
        let render_result = markdown::render_text_with_cache(
            &source,
            absolute_path,
            self.render_options(absolute_path),
        )
        .await
        .inspect_err(|e| tracing::error!("Error rendering markdown: {:?}", e))?;
        self.page(
            absolute_path,
            relative_path,
            render_result,
            PageTemplateChoice::Note,
        )
    }

    /// The markdown render's options for the note at `absolute_path`: build
    /// defaults (`server_mode` off, no transcoding, no review lines).
    fn render_options(&self, absolute_path: &Path) -> markdown::RenderOptions<'_> {
        // Determine if this is an index file (which doesn't need ../ prefix for links)
        let is_index_file = absolute_path
            .file_name()
            .and_then(|f| f.to_str())
            .is_some_and(|f| f == self.config.index_file);
        markdown::RenderOptions {
            root_path: &self.config.root_dir,
            oembed_timeout_ms: self.config.oembed_timeout_ms,
            link_transform_config: LinkTransformConfig {
                markdown_extensions: self.config.markdown_extensions.clone(),
                index_file: self.config.index_file.clone(),
                is_index_file,
                url_depth: None,
                // No repository index for global resolution; the page URL is
                // unused.
                current_page_url: String::new(),
                directory_url_probe: None,
                outside_links: None,
            },
            oembed_cache: Some(Arc::clone(&self.oembed_cache)),
            // Build defaults: transcoding is server-only.
            server_mode: false,
            transcode_enabled: false,
            valid_tag_sources: self.valid_tag_sources.clone(),
            // Hardcoded off: no page and no server for a review anchor to reach.
            review: markdown::ReviewLines::Omit,
            mark_incomplete: self.config.mark_incomplete.unwrap_or(false),
            incomplete_markers: &self.config.incomplete_markers,
            wikilink_index: self.wikilink_index.clone(),
            math_fences: true,
        }
    }

    /// The whole page for a rendered note: the context the server and the
    /// static build give their templates, then the page template `choice`
    /// names or `index.html`.
    fn page(
        &self,
        absolute_path: &Path,
        relative_path: &Path,
        render_result: MarkdownRenderResult,
        choice: PageTemplateChoice<'_>,
    ) -> Result<String, MbrError> {
        let config = &self.config;
        // The same context builder the server and the static build use, so a
        // key the templates read (`sidebar_style`, …) cannot go missing here.
        // A single file has no siblings and no live repository; URLs stay
        // absolute.
        let readability = crate::readability::scores(&crate::readability::ReadabilityCounts {
            words: render_result.word_count,
            sentences: render_result.sentence_count,
            syllables: render_result.syllable_count,
        });
        let file_path = relative_path.to_string_lossy();
        let modified_secs = std::fs::metadata(absolute_path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        let mut extra_context = page_context::markdown_extra_context(
            &page_context::MarkdownPageParams {
                breadcrumb_path: relative_path,
                headings: &render_result.headings,
                has_h1: render_result.has_h1,
                word_count: render_result.word_count,
                readability: &readability,
                file_path: &file_path,
                modified_secs,
                current_url: "",
                siblings: &[],
                slides: render_result.slides.as_ref().map(|s| &s.settings),
                // A single file has no repository index.
                calendar_nav: None,
                frontmatter: Some(&render_result.frontmatter_value),
                // No repository: the render's links, targets assumed to exist.
                frontmatter_links: Some(&render_result.frontmatter_links),
            },
            &page_context::MarkdownContextOptions {
                tag_sources: &config.tag_sources,
                sidebar_style: &config.sidebar_style,
                sidebar_max_items: config.sidebar_max_items,
                graph_depth: config.graph_depth,
                flashcards_concentric_threshold: config.flashcards_concentric_threshold,
                flashcards_progress_indicators: config.flashcards_progress_indicators,
                // No server: no task browser, and the render emitted no
                // `data-mbr-line`.
                tasks_enabled: false,
                review_enabled: false,
                tasks_default_include: config.tasks_default_include,
                title_prefix: &config.title_prefix,
                title_suffix: &config.title_suffix,
                root_dir: None,
                slides: &config.slides,
                calendar: &self.calendar,
            },
            &UrlMode::Absolute,
        );
        let mut frontmatter = render_result.frontmatter;
        frontmatter.insert("server_mode".to_string(), false.into());
        // A contact note still gets its card; with no repository there is no
        // relationship index to link `company` through, and no backlinks.
        page_context::insert_contact(
            &mut extra_context,
            render_result.contact,
            &[],
            &self.relation_registry,
            None,
            &UrlMode::Absolute,
        );

        let page_template = match choice {
            PageTemplateChoice::Note => match page_template::requested_template(&frontmatter) {
                None => None,
                Some(requested) => Some(self.resolve(absolute_path, relative_path, requested?)?),
            },
            PageTemplateChoice::Unsaved { path, source } => {
                Some(page_template::unsaved_page_template(
                    &self.vault,
                    &path.to_string_lossy().replace('\\', "/"),
                    source,
                    &self.exempt_hidden_dirs,
                )?)
            }
        };
        Ok(match page_template {
            Some(template) => self.templates.render_page_template(
                &template,
                &render_result.html,
                frontmatter,
                extra_context,
            )?,
            None => {
                self.templates
                    .render_markdown(&render_result.html, frontmatter, extra_context)?
            }
        })
    }

    /// [`page_template::resolve_page_template`] for the note at
    /// `absolute_path` (`relative_path` from the root): located by its
    /// canonical path when it exists, so a note reached through a link
    /// resolves as the server resolves it, else by `relative_path` (a note
    /// not saved yet).
    fn resolve(
        &self,
        absolute_path: &Path,
        relative_path: &Path,
        template: &str,
    ) -> Result<page_template::PageTemplate, PageTemplateError> {
        let outside = || PageTemplateError::OutsideRoot {
            template: template.to_string(),
        };
        let note = match absolute_path.canonicalize() {
            Ok(canonical) => self.vault.vault_path(&canonical).map_err(|_| outside())?,
            Err(_) => VaultPath::from_relative_native(relative_path).map_err(|_| outside())?,
        };
        page_template::resolve_page_template(&self.vault, &note, template, &self.exempt_hidden_dirs)
    }
}

/// Which page template [`Renderer::page`] renders through.
enum PageTemplateChoice<'a> {
    /// The note's own `template:`, if any, read from disk.
    Note,
    /// Unsaved text as the template file at `path`.
    Unsaved { path: &'a Path, source: &'a str },
}

/// Renders the markdown file at `absolute_path` (`relative_path` from
/// `config.root_dir`) to a whole page, through its page template when it
/// names one. One-shot: see [`Renderer`] to render more than once.
pub async fn render_note(
    config: &Config,
    absolute_path: &Path,
    relative_path: &Path,
) -> Result<String, MbrError> {
    Renderer::new(config)?
        .render_file(absolute_path, relative_path)
        .await
}

/// Renders `source` as the note at `relative_path` (from `config.root_dir`)
/// to a whole page, exactly as [`render_note`] renders that file holding
/// `source` ([`Renderer::render_source`]). One-shot: it loads the templates
/// on every call, so a caller rendering repeatedly (a live preview) keeps a
/// [`Renderer`] instead.
pub fn render_note_source(
    config: &Config,
    relative_path: &Path,
    source: &str,
) -> Result<String, MbrError> {
    Renderer::new(config)?.render_source(relative_path, source)
}

/// The markdown file a page at `absolute_path` (`relative_path` from the
/// root) shows, as `(absolute, relative)`: the path itself, or a folder's
/// index note (`config.index_file`), or `None` (a folder without one is a
/// listing).
pub fn note_file(
    config: &Config,
    absolute_path: &Path,
    relative_path: &Path,
) -> Option<(PathBuf, PathBuf)> {
    if absolute_path.is_dir() {
        let index = absolute_path.join(&config.index_file);
        index
            .is_file()
            .then(|| (index, relative_path.join(&config.index_file)))
    } else {
        Some((absolute_path.to_path_buf(), relative_path.to_path_buf()))
    }
}

/// A note's page template problem, for a command-line caller that must not
/// produce output from a page the server would answer with its error page.
#[derive(Debug, thiserror::Error)]
#[error("{note}: {message}")]
pub struct PageTemplateProblem {
    /// The note, relative to the repository root, `/`-separated.
    pub note: String,
    /// What went wrong (`template `x` failed to render: …`).
    pub message: String,
}

/// Checks that the note at `absolute_path` (`relative_path` from the root; a
/// note, or a folder whose index note it shows) can be rendered through its page template, by rendering it
/// standalone ([`render_note`]) and discarding the page.
///
/// A note without `template:` costs one frontmatter read (the first 8 KB,
/// more only for a longer frontmatter) and
/// nothing else, and a folder without an index note costs a `stat`.
pub async fn check_page_template(
    config: &Config,
    absolute_path: &Path,
    relative_path: &Path,
) -> Result<(), PageTemplateProblem> {
    let Some((note, relative)) = note_file(config, absolute_path, relative_path) else {
        return Ok(());
    };
    let Ok(metadata) = markdown::extract_metadata_from_file(&note) else {
        // Unreadable: the export reports it in its own terms.
        return Ok(());
    };
    if page_template::requested_template(&metadata.metadata).is_none() {
        return Ok(());
    }
    let label = crate::url_path::path_to_url(&relative);
    render_note(config, &note, &relative)
        .await
        .map(drop)
        .map_err(|error| PageTemplateProblem {
            note: label,
            message: match error {
                MbrError::PageTemplate(problem) => problem.to_string(),
                other => error_chain(&other),
            },
        })
}

/// An error and its sources on one line.
fn error_chain(error: &dyn std::error::Error) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A repository holding `files`, and its config (root canonical, as the
    /// CLI reads it).
    fn repo(files: &[(&str, &str)]) -> (tempfile::TempDir, Config) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join(".mbr")).unwrap();
        for (path, contents) in files {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, contents).unwrap();
        }
        let config = Config {
            root_dir: root,
            oembed_timeout_ms: 0,
            ..Default::default()
        };
        (dir, config)
    }

    async fn check(config: &Config, relative: &str) -> Result<(), PageTemplateProblem> {
        let relative = Path::new(relative);
        check_page_template(config, &config.root_dir.join(relative), relative).await
    }

    #[tokio::test]
    async fn a_note_without_a_template_passes() {
        let (_d, config) = repo(&[("plain.md", "# Plain\n"), ("folder/x.md", "x")]);
        check(&config, "plain.md").await.unwrap();
        // A folder without an index note is a listing: nothing to check.
        check(&config, "folder").await.unwrap();
    }

    #[tokio::test]
    async fn a_working_template_passes_and_renders() {
        let (_d, config) = repo(&[
            (
                "bills/a.md",
                "---\ntitle: A\ntemplate: t.html\nn: 2.5\n---\nBody\n",
            ),
            (
                "bills/t.html",
                "<h1>{{ title }}</h1>{{ frontmatter.n * 2 }}",
            ),
        ]);
        check(&config, "bills/a.md").await.unwrap();
        let html = render_note(
            &config,
            &config.root_dir.join("bills/a.md"),
            Path::new("bills/a.md"),
        )
        .await
        .unwrap();
        assert_eq!(html, "<h1>A</h1>5");
    }

    #[tokio::test]
    async fn a_missing_template_names_the_note_and_the_template() {
        let (_d, config) = repo(&[("bills/a.md", "---\ntemplate: missing.html\n---\nA\n")]);
        let problem = check(&config, "bills/a.md").await.unwrap_err();
        assert_eq!(problem.note, "bills/a.md");
        assert_eq!(
            problem.to_string(),
            "bills/a.md: template `missing.html` was not found"
        );
    }

    #[tokio::test]
    async fn a_broken_or_refused_template_fails() {
        let (_d, config) = repo(&[
            ("a.md", "---\ntemplate: t.html\n---\nA\n"),
            ("t.html", "{{ nope.nope }}"),
            ("b.md", "---\ntemplate: ../outside.html\n---\nB\n"),
        ]);
        let broken = check(&config, "a.md").await.unwrap_err().to_string();
        assert!(
            broken.starts_with("a.md: template `t.html` failed to render: "),
            "{broken}"
        );
        assert!(broken.contains("nope"), "{broken}");
        let refused = check(&config, "b.md").await.unwrap_err().to_string();
        assert_eq!(
            refused,
            "b.md: template `../outside.html` is outside the repository"
        );
    }

    /// A note exercising what differs by location: relative and
    /// root-relative links, an image, a heading, frontmatter.
    const LINKED_NOTE: &str = "---\ntitle: Guide\ntags: [a, b]\n---\n# Guide\n\n\
        See [the other](other.md), [up](../readme.md), [root](/top.md) \
        and ![logo](img/logo.png).\n\n- [ ] a task\n";

    #[tokio::test]
    async fn a_source_renders_exactly_as_the_same_file() {
        for relative in ["docs/guide.md", "docs/index.md", "top.md"] {
            let (_d, config) = repo(&[(relative, LINKED_NOTE)]);
            let renderer = Renderer::new(&config).unwrap();
            let from_disk = renderer
                .render_file(&config.root_dir.join(relative), Path::new(relative))
                .await
                .unwrap();
            let from_source = renderer
                .render_source(Path::new(relative), LINKED_NOTE)
                .unwrap();
            assert_eq!(from_source, from_disk, "{relative}");
            // The one-shot functions are the same renders.
            let one_shot = render_note_source(&config, Path::new(relative), LINKED_NOTE).unwrap();
            assert_eq!(one_shot, from_disk, "{relative}");
            let one_shot_file = render_note(
                &config,
                &config.root_dir.join(relative),
                Path::new(relative),
            )
            .await
            .unwrap();
            assert_eq!(one_shot_file, from_disk, "{relative}");
        }
    }

    #[test]
    fn links_resolve_relative_to_the_given_path() {
        let (_d, config) = repo(&[]);
        let renderer = Renderer::new(&config).unwrap();
        // `docs/guide.md` is served at `/docs/guide/`, so a sibling is one
        // level up from the page URL.
        let guide = renderer
            .render_source(Path::new("docs/guide.md"), LINKED_NOTE)
            .unwrap();
        assert!(guide.contains(r#"href="../other/""#), "{guide}");
        assert!(guide.contains(r#"src="../img/logo.png""#), "{guide}");
        // An index note is served at its folder's URL: no compensation.
        let index = renderer
            .render_source(Path::new("docs/index.md"), LINKED_NOTE)
            .unwrap();
        assert!(index.contains(r#"href="other/""#), "{index}");
        assert!(index.contains(r#"src="img/logo.png""#), "{index}");
    }

    #[test]
    fn a_source_with_a_template_is_rendered_through_it() {
        let (_d, config) = repo(&[(
            "bills/t.html",
            "<h1>{{ title }}</h1>{{ frontmatter.n * 2 }}|{{ markdown | safe }}",
        )]);
        let renderer = Renderer::new(&config).unwrap();
        // The note was never saved: it is located by its relative path.
        let html = renderer
            .render_source(
                Path::new("bills/new.md"),
                "---\ntitle: New\ntemplate: t.html\nn: 4\n---\nSee [x](x.md)\n",
            )
            .unwrap();
        assert_eq!(
            html,
            "<h1>New</h1>8|<section>\n<p>See <a href=\"../x/\">x</a></p>\n</section>\n"
        );
    }

    #[test]
    fn a_template_problem_in_a_source_is_an_error() {
        let (_d, config) = repo(&[]);
        let renderer = Renderer::new(&config).unwrap();
        let error = renderer
            .render_source(Path::new("a.md"), "---\ntemplate: missing.html\n---\nA\n")
            .unwrap_err();
        assert!(
            matches!(
                error,
                MbrError::PageTemplate(PageTemplateError::NotFound { .. })
            ),
            "{error:?}"
        );
    }

    #[test]
    fn a_kept_renderer_sees_page_template_edits_on_the_next_render() {
        let (_d, config) = repo(&[("t.html", "one {{ title }}")]);
        let renderer = Renderer::new(&config).unwrap();
        let source = "---\ntitle: T\ntemplate: t.html\n---\n";
        assert_eq!(
            renderer.render_source(Path::new("a.md"), source).unwrap(),
            "one T"
        );
        std::fs::write(config.root_dir.join("t.html"), "two {{ title }}").unwrap();
        assert_eq!(
            renderer.render_source(Path::new("a.md"), source).unwrap(),
            "two T"
        );
    }

    /// The invoicer's layout: notes in `invoices/`, templates in
    /// `invoices/tpl/`, named root-relative.
    fn invoice_repo() -> (tempfile::TempDir, Config) {
        repo(&[
            (
                "invoices/tpl/invoice.html",
                "saved {{ title }} {% include \"lines.html\" %}",
            ),
            ("invoices/tpl/lines.html", "[lines {{ frontmatter.n }}]"),
        ])
    }

    const INVOICE: &str =
        "---\ntitle: Inv\ntemplate: /invoices/tpl/invoice.html\nn: 3\n---\nBody\n";

    #[test]
    fn an_unsaved_template_renders_with_saved_partials() {
        let (_d, config) = invoice_repo();
        let renderer = Renderer::new(&config).unwrap();
        let note = Path::new("invoices/2026-001.md");
        let html = renderer
            .render_source_with_template(
                note,
                INVOICE,
                Path::new("/invoices/tpl/invoice.html"),
                "draft {{ title }} {% include \"lines.html\" %} {{ markdown | safe }}",
            )
            .unwrap();
        assert_eq!(
            html,
            "draft Inv [lines 3] <section>\n<p>Body</p>\n</section>\n"
        );
        // The saved template is untouched and still what a plain render uses.
        assert_eq!(
            renderer.render_source(note, INVOICE).unwrap(),
            "saved Inv [lines 3]"
        );
        // The note's own `template:` is not consulted: a note naming another
        // template, or none, renders through the given text too.
        let plain = renderer
            .render_source_with_template(
                Path::new("invoices/plain.md"),
                "---\ntitle: P\n---\n",
                Path::new("invoices/tpl/new.html"),
                "new {{ title }}",
            )
            .unwrap();
        assert_eq!(plain, "new P");
    }

    #[test]
    fn a_broken_unsaved_template_is_an_error_and_poisons_nothing() {
        let (_d, config) = invoice_repo();
        let renderer = Renderer::new(&config).unwrap();
        let note = Path::new("invoices/2026-001.md");
        let template = Path::new("invoices/tpl/invoice.html");
        for (draft, expected) in [
            // Syntax error: Tera names the place.
            ("{% if title %}unclosed", " --> 1:"),
            // Render error: an unknown variable.
            ("{{ nope.nope }}", "nope"),
            // An include of a partial that is not there.
            ("{% include \"missing.html\" %}", "missing.html"),
        ] {
            let error = renderer
                .render_source_with_template(note, INVOICE, template, draft)
                .unwrap_err();
            let MbrError::PageTemplate(problem @ PageTemplateError::Render { .. }) = &error else {
                panic!("{draft}: {error:?}");
            };
            let message = problem.to_string();
            assert!(
                message.starts_with("template `/invoices/tpl/invoice.html` failed to render: "),
                "{message}"
            );
            assert!(message.contains(expected), "{draft}: {message}");
            // The next render, saved or unsaved, is unaffected.
            assert_eq!(
                renderer.render_source(note, INVOICE).unwrap(),
                "saved Inv [lines 3]"
            );
            assert_eq!(
                renderer
                    .render_source_with_template(note, INVOICE, template, "ok {{ title }}")
                    .unwrap(),
                "ok Inv"
            );
        }
        // A refused path is an error too, before anything is rendered.
        let refused = renderer
            .render_source_with_template(note, INVOICE, Path::new(".mbr/x.html"), "x")
            .unwrap_err();
        assert!(
            matches!(
                refused,
                MbrError::PageTemplate(PageTemplateError::Hidden { .. })
            ),
            "{refused:?}"
        );
    }

    #[test]
    fn mbr_template_overrides_apply_and_reload_on_request() {
        let (_d, config) = repo(&[(".mbr/index.html", "v1 {{ title }}")]);
        let renderer = Renderer::new(&config).unwrap();
        let source = "---\ntitle: T\n---\nBody\n";
        assert_eq!(
            renderer.render_source(Path::new("a.md"), source).unwrap(),
            "v1 T"
        );
        std::fs::write(config.root_dir.join(".mbr/index.html"), "v2 {{ title }}").unwrap();
        // Loaded once: unchanged until reloaded.
        assert_eq!(
            renderer.render_source(Path::new("a.md"), source).unwrap(),
            "v1 T"
        );
        renderer.reload_templates().unwrap();
        assert_eq!(
            renderer.render_source(Path::new("a.md"), source).unwrap(),
            "v2 T"
        );
    }

    #[test]
    fn a_renderer_is_shareable_across_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Renderer>();
    }

    #[tokio::test]
    async fn a_folder_is_checked_through_its_index_note() {
        let (_d, config) = repo(&[("docs/index.md", "---\ntemplate: gone.html\n---\nD\n")]);
        let problem = check(&config, "docs").await.unwrap_err();
        assert_eq!(problem.note, "docs/index.md");
    }
}
