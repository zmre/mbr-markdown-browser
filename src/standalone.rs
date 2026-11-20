//! One note rendered to a whole HTML page with no server and no repository
//! index: what `mbr -o` prints, and what `mbr --pdf` checks a note's page
//! template with before it starts an export ([`check_page_template`]).
//!
//! The page is the one the server would send, minus what needs a server: no
//! siblings, no wikilink index, absolute URLs. A note with its own page
//! template (`template:`) is rendered through it, resolved inside the
//! repository exactly as the server and the static build resolve it
//! ([`crate::page_template::resolve_page_template`]); a problem is an error,
//! never a silent fallback to the normal page.

use std::path::{Path, PathBuf};

use crate::Config;
use crate::errors::MbrError;
use crate::link_transform::LinkTransformConfig;
use crate::markdown;
use crate::page_template::{self, PageTemplateError};
use crate::templates::Templates;
use crate::vault::{LocalVault, Vault, VaultPath};

/// Renders the markdown file at `absolute_path` (`relative_path` from
/// `config.root_dir`) to a whole page, through its page template when it
/// names one.
pub async fn render_note(
    config: &Config,
    absolute_path: &Path,
    relative_path: &Path,
) -> Result<String, MbrError> {
    // Determine if this is an index file (which doesn't need ../ prefix for links)
    let is_index_file = absolute_path
        .file_name()
        .and_then(|f| f.to_str())
        .is_some_and(|f| f == config.index_file);

    let link_transform_config = LinkTransformConfig {
        markdown_extensions: config.markdown_extensions.clone(),
        index_file: config.index_file.clone(),
        is_index_file,
        url_depth: None,
        // A single file with no repo index, so body wikilinks never resolve
        // globally; the page URL is unused.
        current_page_url: String::new(),
        directory_url_probe: None,
        outside_links: None,
    };

    // server_mode=false, transcode disabled (transcode is server-only); mirrors
    // build defaults (off unless explicitly enabled).
    let valid_tag_sources = crate::config::tag_sources_to_set(&config.tag_sources);
    let mark_incomplete = config.mark_incomplete.unwrap_or(false);
    let render_result = markdown::render(
        absolute_path.to_path_buf(),
        config.root_dir.as_path(),
        config.oembed_timeout_ms,
        link_transform_config,
        false, // server_mode
        false, // transcode
        valid_tag_sources,
        // Hardcoded off: no page and no server for a review anchor to reach.
        markdown::ReviewLines::Omit,
        mark_incomplete,
        &config.incomplete_markers,
        None, // no repo wikilink index
    )
    .await
    .inspect_err(|e| tracing::error!("Error rendering markdown: {:?}", e))?;
    let templates = Templates::new(&config.root_dir, config.template_folder.as_deref())
        .inspect_err(|e| tracing::error!("Error parsing template: {e}"))?;
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
    let mut extra_context = crate::page_context::markdown_extra_context(
        &crate::page_context::MarkdownPageParams {
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
        &crate::page_context::MarkdownContextOptions {
            tag_sources: &config.tag_sources,
            sidebar_style: &config.sidebar_style,
            sidebar_max_items: config.sidebar_max_items,
            graph_depth: config.graph_depth,
            flashcards_concentric_threshold: config.flashcards_concentric_threshold,
            flashcards_progress_indicators: config.flashcards_progress_indicators,
            // No server: no task browser, and the render above emitted no
            // `data-mbr-line`.
            tasks_enabled: false,
            review_enabled: false,
            tasks_default_include: config.tasks_default_include,
            title_prefix: &config.title_prefix,
            title_suffix: &config.title_suffix,
            root_dir: None,
            slides: &config.slides,
            calendar: &crate::page_context::CalendarChrome::new(&config.calendar),
        },
        &crate::page_context::UrlMode::Absolute,
    );
    let mut frontmatter = render_result.frontmatter;
    frontmatter.insert("server_mode".to_string(), false.into());
    // A contact note still gets its card; with no repository there is no
    // relationship index to link `company` through, and no backlinks.
    crate::page_context::insert_contact(
        &mut extra_context,
        render_result.contact,
        &[],
        &crate::relationships::RelationTypeRegistry::from_types(&config.relationship_types),
        None,
        &crate::page_context::UrlMode::Absolute,
    );

    let page_template = match page_template::requested_template(&frontmatter) {
        None => None,
        Some(requested) => Some(resolve(config, absolute_path, requested?)?),
    };
    Ok(match page_template {
        Some(template) => templates.render_page_template(
            &template,
            &render_result.html,
            frontmatter,
            extra_context,
        )?,
        None => templates.render_markdown(&render_result.html, frontmatter, extra_context)?,
    })
}

/// [`page_template::resolve_page_template`] for the note at `absolute_path`
/// in a plain [`LocalVault`] over `config.root_dir`.
fn resolve(
    config: &Config,
    absolute_path: &Path,
    template: &str,
) -> Result<page_template::PageTemplate, PageTemplateError> {
    let vault = LocalVault::new(&config.root_dir);
    let canonical_note = absolute_path
        .canonicalize()
        .unwrap_or_else(|_| absolute_path.to_path_buf());
    let note = vault
        .vault_path(&canonical_note)
        .map_err(|_| PageTemplateError::OutsideRoot {
            template: template.to_string(),
        })?;
    let exempt: Vec<VaultPath> = config
        .explicit_hidden_dirs
        .iter()
        .filter_map(|dir| VaultPath::from_relative_native(dir).ok())
        .collect();
    page_template::resolve_page_template(&vault, &note, template, &exempt)
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
/// A note without `template:` costs one frontmatter read (the first 8 KB) and
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

    #[tokio::test]
    async fn a_folder_is_checked_through_its_index_note() {
        let (_d, config) = repo(&[("docs/index.md", "---\ntemplate: gone.html\n---\nD\n")]);
        let problem = check(&config, "docs").await.unwrap_err();
        assert_eq!(problem.note, "docs/index.md");
    }
}
