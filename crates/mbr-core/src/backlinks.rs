//! Building and maintaining the repository-wide backlink index
//! ([`InboundIndex`]) from a scanned [`Repo`].
//!
//! The server (`mbr_server::server`) and the library façade
//! ([`crate::site::Site`]) both keep their backlinks with these functions, so
//! the two cannot disagree about which links produce a backlink: each page is
//! parsed with the renderer's own link collection
//! ([`markdown::extract_outbound_links_from_vault`], frontmatter links
//! included) and resolved with [`resolve_outbound_links`], exactly as the
//! page's `links.json` is.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::change_event::{ChangeEventType, FileChangeEvent};
use crate::link_index::{InboundIndex, resolve_outbound_links};
use crate::link_transform::{DirectoryUrlProbe, LinkTransformConfig};
use crate::markdown;
use crate::outside_link::OutsideLinkContext;
use crate::repo::Repo;

/// Everything [`index_page_links`] needs to reproduce the renderer's link
/// resolution outside a request.
///
/// Cloned once at startup rather than read per request, because the index is
/// built and maintained from background tasks.
#[derive(Clone)]
pub struct BacklinkIndexConfig {
    pub base_dir: PathBuf,
    pub index_file: String,
    pub markdown_extensions: Vec<String>,
    pub valid_tag_sources: HashSet<String>,
    /// The page render's own probe (built from the same resolver
    /// configuration), so an extension-less or root-relative link is
    /// canonicalized here exactly as on the page.
    pub directory_url_probe: DirectoryUrlProbe,
    /// The page render's outside-link context, so the index records a mounted
    /// target's in-vault URL as the page does.
    pub outside_links: OutsideLinkContext,
}

impl BacklinkIndexConfig {
    /// The configuration for `repo` as its own settings describe it: the
    /// repository's resolver for the probe, and `outside_links` as given (the
    /// server passes one naming absolute paths).
    pub fn for_repo(repo: &Repo, outside_links: OutsideLinkContext) -> Self {
        Self {
            base_dir: repo.vault().root().to_path_buf(),
            index_file: repo.index_file.clone(),
            markdown_extensions: repo.markdown_extensions().to_vec(),
            valid_tag_sources: crate::config::tag_sources_to_set(repo.tag_sources()),
            directory_url_probe: crate::link_transform::filesystem_directory_url_probe(
                repo.resolver_config(),
            ),
            outside_links,
        }
    }

    fn page_config(&self, path: &Path, url_path: &str) -> LinkTransformConfig {
        let is_index_file = path
            .file_name()
            .and_then(|f| f.to_str())
            .is_some_and(|f| f == self.index_file);
        LinkTransformConfig {
            markdown_extensions: self.markdown_extensions.clone(),
            index_file: self.index_file.clone(),
            is_index_file,
            url_depth: None,
            current_page_url: url_path.to_string(),
            directory_url_probe: Some(self.directory_url_probe.clone()),
            outside_links: Some(self.outside_links.clone()),
        }
    }

    fn is_markdown(&self, path: &Path) -> bool {
        path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
            self.markdown_extensions
                .iter()
                .any(|ext| e.eq_ignore_ascii_case(ext))
        })
    }
}

/// Parse one markdown file and record its contribution to the backlink index.
///
/// Link resolution must match what a real render would produce, or the info
/// panel would show backlinks the page does not actually contain. That is why
/// this goes through the same `extract_outbound_links_from_vault` →
/// `resolve_outbound_links` pair the request path uses, with the same
/// `LinkTransformConfig`, rather than re-deriving URLs.
pub fn index_page_links(
    repo: &Repo,
    index: &InboundIndex,
    cfg: &BacklinkIndexConfig,
    path: &Path,
    url_path: &str,
) {
    let link_transform_config = cfg.page_config(path, url_path);
    match markdown::extract_outbound_links_from_vault(
        repo.vault().as_ref(),
        path,
        &cfg.base_dir,
        link_transform_config.clone(),
        true, // server_mode
        cfg.valid_tag_sources.clone(),
        Some(repo.wikilink_index.clone()),
    ) {
        Ok(links) => {
            // The same resolution `links.json` publishes, so a backlink is
            // recorded exactly where the page's outbound list says it points.
            let resolved = resolve_outbound_links(links, &link_transform_config);
            index.set_page_links(url_path, &resolved);
        }
        Err(e) => {
            // A file that cannot be parsed contributes no backlinks. Withdraw
            // whatever it contributed before so a file that became unreadable
            // does not leave stale backlinks behind.
            // A not-downloaded note (a cloud provider evicted it) is an
            // expected state on mobile, not something to warn about.
            if e.is_not_downloaded() {
                tracing::debug!("Backlink index: {} not downloaded yet: {e}", path.display());
            } else {
                tracing::warn!("Backlink index: failed to read {}: {e}", path.display());
            }
            index.remove_page(url_path);
        }
    }
}

/// Build the whole backlink index from the current repository contents, and
/// mark it ready.
///
/// Blocking and rayon-parallel; call from `spawn_blocking` in async code.
pub fn populate_inbound_index(repo: &Repo, index: &InboundIndex, cfg: &BacklinkIndexConfig) {
    use rayon::prelude::*;

    let pages: Vec<(PathBuf, String)> = repo
        .markdown_files
        .pin()
        .iter()
        .map(|(path, info)| (path.clone(), info.url_path.clone()))
        .collect();

    let page_count = pages.len();
    let started = Instant::now();

    pages.par_iter().for_each(|(path, url_path)| {
        index_page_links(repo, index, cfg, path, url_path);
    });

    index.mark_ready();
    tracing::info!(
        "Backlink index built in {:?}: {} pages parsed, {} pages have backlinks",
        started.elapsed(),
        page_count,
        index.target_count(),
    );
}

/// Applies a batch of file changes to `repo` and every index derived from it,
/// file by file: the repository's maps ([`Repo::invalidate_file`]), the tag,
/// relationship and wikilink indexes, and — once it is built — the backlink
/// index, for the pages that changed.
///
/// `on_file` is called after each file's [`Repo::invalidate_file`], for an
/// index the caller keeps beside these (the server's task index).
///
/// Blocking; the caller serializes it against [`populate_inbound_index`]
/// (the server holds a lock across both), since an edit interleaving with the
/// initial build could be undone by it.
pub fn apply_file_changes(
    repo: &Repo,
    inbound_index: &InboundIndex,
    cfg: &BacklinkIndexConfig,
    events: &[FileChangeEvent],
    mut on_file: impl FnMut(&Path, &ChangeEventType),
) {
    let has_tag_changes = events.iter().any(|e| {
        matches!(
            e.event,
            ChangeEventType::Deleted | ChangeEventType::Modified
        )
    });
    let mut index_targets: Vec<(PathBuf, Option<String>, bool)> = Vec::new();
    for event in events {
        if event.path.is_empty() {
            continue;
        }
        let abs_path = PathBuf::from(&event.path);
        index_targets.push((
            abs_path.clone(),
            // Take the URL from the repository's own map rather than
            // recomputing it. `base_dir` is the path as configured, while the
            // scanner keys everything by the *canonical* root, and on macOS a
            // temp dir differs (`/var` vs `/private/var`). A recomputed URL
            // would not match the key the initial build inserted under, so
            // the edit would write a second entry and never withdraw the
            // stale one. Captured before `invalidate_file` because a deletion
            // removes the entry it is read from.
            repo.markdown_files
                .pin()
                .get(&abs_path)
                .map(|info| info.url_path.clone()),
            matches!(event.event, ChangeEventType::Deleted),
        ));
        repo.invalidate_file(&abs_path, &event.event);
        // After `repo.invalidate_file`, never before: a created or modified
        // file's url/title are read back out of the repository's own map,
        // which the call above is what populates.
        on_file(&abs_path, &event.event);
    }
    // Rebuild tag index if any files were deleted or modified (created files
    // add tags inline in invalidate_file).
    if has_tag_changes {
        repo.rebuild_tag_index();
    }
    // Relationships may have changed on any create/modify/delete; rebuild the
    // index so endpoint resolution stays consistent.
    repo.build_relationship_index();
    // The global wikilink index must track the same changes.
    repo.build_wikilink_index();

    // Re-index only the pages that changed. This runs after
    // build_wikilink_index() on purpose: `[[Name]]` links resolve through
    // that index, so re-extracting first would record links against the
    // pre-edit name table. Until the initial build has run there is nothing
    // to patch: it will read every page as it is then.
    if !inbound_index.is_ready() {
        return;
    }
    for (abs_path, known_url, is_delete) in &index_targets {
        if !cfg.is_markdown(abs_path) {
            continue;
        }
        let url_path = match known_url {
            Some(url) => url.clone(),
            // Only reachable for a file the scan never saw (created inside
            // this batch, or deleted before it was ever indexed).
            None => crate::repo::build_markdown_url_path(abs_path, &cfg.base_dir, &cfg.index_file),
        };
        if *is_delete {
            inbound_index.remove_page(&url_path);
        } else {
            index_page_links(repo, inbound_index, cfg, abs_path, &url_path);
        }
    }
}
