//! Links written as frontmatter values.
//!
//! A frontmatter string whose **whole** value is a wikilink
//! (`customer: "[[Customer X]]"`, `[[Target|Label]]`) or a path to a file
//! (`/`-rooted, `./` or `../`-relative: `contract: "/customers/MSA.pdf"`) is a
//! link, at the top level or as an element of a list of strings
//! (`contracts: ["[[MSA 2026]]", "./SOW 3 (signed).pdf"]`). Nothing else is:
//! a value with text around the link, a value inside a nested map, a list of
//! maps, or an ordinary string that merely contains a `/` stays text.
//!
//! The pieces, in the order a value goes through them:
//!
//! 1. [`find_frontmatter_links`] reads the raw YAML once (at scan and at
//!    render) and yields [`FrontmatterLinkSource`]s. Pure, no index needed, and
//!    serializable, so a repository snapshot can carry them.
//! 2. [`frontmatter_outbound_links`] turns them into the [`OutboundLink`]s a
//!    page collects, with [`LinkOrigin::Frontmatter`]: the renderer appends
//!    them to the body's links, so `links.json`, the server's backlink index
//!    and the static build's inversion all see them with no second path. A
//!    wikilink goes through [`crate::markdown::link_href`], exactly as a body
//!    `[[Name]]` does (current folder first, then title, alias, stem), and a
//!    path through [`crate::link_transform`] like `[x](./path)`.
//! 3. [`resolve_frontmatter_links`] resolves them to site-absolute targets
//!    with [`resolve_outbound_links`] — the one resolution `links.json`
//!    publishes — and asks a [`LinkTargetLookup`] whether each target exists
//!    and what it is called. [`crate::repo::Repo`] is the lookup the server,
//!    the build and `mbr_core::Site` use: the path resolver, i.e. what the
//!    server would serve. [`AssumeTargetsExist`] is for callers with no
//!    repository (CLI, QuickLook).
//!
//! Some keys are never links: `template` (the page template, see
//! [`crate::page_template`]), `relationships` (typed edges, see
//! [`crate::relationships`]) and the private contact fields
//! ([`crate::contact::is_public_frontmatter_key`]), which must not leak into
//! `site.json` through a link either.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
pub use yaml_rust2::Yaml;

use crate::link_index::{
    LinkOrigin, OutboundLink, is_internal_link, resolve_outbound_links, split_url_anchor,
};
use crate::link_transform::{LinkTransformConfig, make_relative_url};
use crate::wikilink_index::WikilinkIndex;

/// Frontmatter keys whose values are never links, compared ASCII
/// case-insensitively. See the module docs.
const NOT_LINK_KEYS: &[&str] = &["template", "relationships"];

/// How a frontmatter link was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FrontmatterLinkForm {
    /// `[[Target]]` or `[[Target|Label]]`.
    Wikilink,
    /// `/x/y.pdf`, `./y.pdf` or `../y.md`.
    Path,
}

/// A frontmatter value that is a link, before resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontmatterLinkSource {
    /// The frontmatter key, as authored.
    pub key: String,
    /// The value as authored, trimmed (`[[Customer X]]`).
    pub raw: String,
    /// The element's index when the value is a list, `None` for a scalar.
    pub position: Option<usize>,
    /// Wikilink or path.
    pub form: FrontmatterLinkForm,
    /// What the link names: a wikilink's target (`Customer X`, `#anchor`
    /// kept), or the path as written.
    pub destination: String,
    /// A wikilink's `|Label`, trimmed; `None` when absent or empty.
    pub label: Option<String>,
}

impl FrontmatterLinkSource {
    /// The value's own name: a wikilink's target (anchor dropped) or a path's
    /// file name; the raw value if that is empty.
    pub fn name(&self) -> String {
        let (without_anchor, _) = split_url_anchor(&self.destination);
        let name = match self.form {
            FrontmatterLinkForm::Wikilink => without_anchor.trim(),
            FrontmatterLinkForm::Path => without_anchor
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or_default(),
        };
        if name.is_empty() {
            self.raw.clone()
        } else {
            name.to_string()
        }
    }

    /// The text a reader sees when the target's name is not known: the
    /// label, else [`Self::name`].
    pub fn display_text(&self) -> String {
        self.label.clone().unwrap_or_else(|| self.name())
    }

    fn origin(&self) -> LinkOrigin {
        LinkOrigin::Frontmatter {
            key: self.key.clone(),
        }
    }
}

/// Whether `key`'s values may be links. See the module docs.
pub fn is_link_key(key: &str) -> bool {
    !NOT_LINK_KEYS.iter().any(|k| k.eq_ignore_ascii_case(key))
        && crate::contact::is_public_frontmatter_key(key)
}

/// Parses one string value: `Some((form, destination, label))` when the whole
/// value is a frontmatter link.
fn parse_link_value(value: &str) -> Option<(FrontmatterLinkForm, String, Option<String>)> {
    let value = value.trim();
    if value.contains(['\n', '\r']) {
        return None;
    }
    if let Some(inner) = value.strip_prefix("[[").and_then(|r| r.strip_suffix("]]")) {
        // One wikilink, not `[[a]] and [[b]]`.
        if inner.contains("[[") || inner.contains("]]") {
            return None;
        }
        let (target, label) = match inner.split_once('|') {
            Some((target, label)) => (target.trim(), Some(label.trim())),
            None => (inner.trim(), None),
        };
        if target.is_empty() || target.starts_with('#') {
            return None;
        }
        let label = label.filter(|l| !l.is_empty()).map(str::to_string);
        return Some((FrontmatterLinkForm::Wikilink, target.to_string(), label));
    }
    let is_path = (value.starts_with('/') && !value.starts_with("//"))
        || value.starts_with("./")
        || value.starts_with("../");
    // `/` alone names the site root, not a file; the rule is about files.
    (is_path && !value.trim_matches(['/', '.']).is_empty())
        .then(|| (FrontmatterLinkForm::Path, value.to_string(), None))
}

/// Every frontmatter link in a note's raw frontmatter, in key order and list
/// order. Empty for anything but a YAML mapping.
pub fn find_frontmatter_links(yaml: &Yaml) -> Vec<FrontmatterLinkSource> {
    let Some(hash) = yaml.as_hash() else {
        return Vec::new();
    };
    hash.iter()
        .filter_map(|(k, v)| Some((k.as_str()?, v)))
        .filter(|(key, _)| is_link_key(key))
        .flat_map(|(key, value)| {
            let source = |raw: &str, position: Option<usize>| {
                parse_link_value(raw).map(|(form, destination, label)| FrontmatterLinkSource {
                    key: key.to_string(),
                    raw: raw.trim().to_string(),
                    position,
                    form,
                    destination,
                    label,
                })
            };
            match value {
                Yaml::String(s) => source(s, None).into_iter().collect::<Vec<_>>(),
                Yaml::Array(items) => items
                    .iter()
                    .enumerate()
                    .filter_map(|(i, item)| item.as_str().and_then(|s| source(s, Some(i))))
                    .collect(),
                _ => Vec::new(),
            }
        })
        .collect()
}

/// The [`OutboundLink`]s `sources` stand for on the page `page`, **before**
/// [`resolve_outbound_links`] — the form the renderer collects body links in,
/// so the two go through every later step together.
///
/// A wikilink is written exactly as [`crate::markdown::link_href`] writes a
/// body `[[Name]]`: its global-fallback URL when the index has one, else the
/// name for the relative transform. A path is recorded as written.
pub fn frontmatter_outbound_links(
    sources: &[FrontmatterLinkSource],
    page: &LinkTransformConfig,
    valid_tag_sources: &HashSet<String>,
    wikilink_index: Option<&WikilinkIndex>,
) -> Vec<OutboundLink> {
    sources
        .iter()
        .map(|source| {
            let destination = match source.form {
                FrontmatterLinkForm::Wikilink => crate::markdown::link_href(
                    &source.destination,
                    true,
                    page,
                    valid_tag_sources,
                    wikilink_index,
                )
                .global
                .unwrap_or_else(|| tag_link_or(&source.destination, valid_tag_sources)),
                FrontmatterLinkForm::Path => source.destination.clone(),
            };
            let (to, anchor) = split_url_anchor(&destination);
            OutboundLink {
                internal: is_internal_link(&destination),
                to,
                text: source.display_text(),
                anchor,
                origin: Some(source.origin()),
            }
        })
        .collect()
}

/// A tag wikilink (`[[Tags:rust]]`) names its tag page; anything else is left
/// for the relative transform, as the renderer leaves a body wikilink.
fn tag_link_or(destination: &str, valid_tag_sources: &HashSet<String>) -> String {
    crate::wikilink::parse_tag_link(destination, valid_tag_sources)
        .map(|tag| tag.url_path())
        .unwrap_or_else(|| destination.to_string())
}

/// What a [`LinkTargetLookup`] knows about an existing target.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LinkTargetInfo {
    /// Its name: a note's title, a file's or folder's name. `None` when the
    /// lookup cannot tell, and the link's own text is used.
    pub title: Option<String>,
}

/// Answers whether a site URL names something that exists, and what it is
/// called.
pub trait LinkTargetLookup {
    /// `None` when nothing is served at `site_url` (site-absolute, decoded,
    /// e.g. `/customers/customer-x/` or `/contracts/MSA (signed).pdf`).
    fn lookup(&self, site_url: &str) -> Option<LinkTargetInfo>;
}

/// The lookup for callers with no repository (CLI, QuickLook): every target
/// exists and is named by its link's text.
#[derive(Debug, Clone, Copy, Default)]
pub struct AssumeTargetsExist;

impl LinkTargetLookup for AssumeTargetsExist {
    fn lookup(&self, _site_url: &str) -> Option<LinkTargetInfo> {
        Some(LinkTargetInfo::default())
    }
}

/// A target that exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedLink {
    /// Site-absolute URL (`/customers/customer-x/`).
    pub url: String,
    /// The target's title (a note's), or file or folder name.
    pub title: String,
}

/// A frontmatter link resolved against its page and the site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontmatterLink {
    /// The frontmatter key.
    pub key: String,
    /// The value as authored, trimmed.
    pub raw: String,
    /// The index in a list value, `None` for a scalar.
    pub position: Option<usize>,
    /// Wikilink or path.
    pub form: FrontmatterLinkForm,
    /// A wikilink's `|Label`.
    pub label: Option<String>,
    /// The value's own name ([`FrontmatterLinkSource::name`]): what the link
    /// is called when its target is missing or has no title.
    pub name: String,
    /// The site-absolute URL the value points at, exactly as `links.json`
    /// records it — also when nothing is there.
    pub target: String,
    /// `#fragment` of the target, if any.
    pub anchor: Option<String>,
    /// `false` for a path that leaves the repository (see
    /// [`crate::outside_link`]); such a link has no target here.
    pub internal: bool,
    /// The target, when it exists.
    pub resolved: Option<ResolvedLink>,
}

impl FrontmatterLink {
    /// The target's title when it exists, else the value's own name.
    pub fn title(&self) -> &str {
        self.resolved
            .as_ref()
            .map_or(self.name.as_str(), |r| r.title.as_str())
    }

    /// The reader-facing text: the label, else [`Self::title`].
    pub fn text(&self) -> &str {
        self.label.as_deref().unwrap_or_else(|| self.title())
    }
}

/// Resolves `sources` (from a note at `page`) to site-absolute targets and
/// checks each against `lookup`.
///
/// The targets are [`frontmatter_outbound_links`] put through
/// [`resolve_outbound_links`], so a link's `target` is by construction what
/// the page's `links.json` lists and where its backlink is recorded.
pub fn resolve_frontmatter_links(
    sources: &[FrontmatterLinkSource],
    page: &LinkTransformConfig,
    valid_tag_sources: &HashSet<String>,
    wikilink_index: Option<&WikilinkIndex>,
    lookup: &dyn LinkTargetLookup,
) -> Vec<FrontmatterLink> {
    let outbound = frontmatter_outbound_links(sources, page, valid_tag_sources, wikilink_index);
    // `resolve_outbound_links` drops a link with an empty target; none of
    // these has one (a source always names something), so the zip is aligned.
    let resolved = resolve_outbound_links(outbound, page);
    debug_assert_eq!(resolved.len(), sources.len());
    let mut links: Vec<FrontmatterLink> = sources
        .iter()
        .zip(resolved)
        .map(|(source, link)| FrontmatterLink {
            key: source.key.clone(),
            raw: source.raw.clone(),
            position: source.position,
            form: source.form,
            label: source.label.clone(),
            name: source.name(),
            target: link.to,
            anchor: link.anchor,
            internal: link.internal,
            resolved: None,
        })
        .collect();
    check_targets(&mut links, lookup);
    links
}

/// Re-checks already resolved links against `lookup`: a render has no
/// repository and resolves with [`AssumeTargetsExist`]
/// ([`crate::markdown::MarkdownRenderResult::frontmatter_links`]); a caller
/// that has one (the server, the build) sets the real answer here.
pub fn check_targets(links: &mut [FrontmatterLink], lookup: &dyn LinkTargetLookup) {
    for link in links {
        link.resolved = link
            .internal
            .then(|| lookup.lookup(&link.target))
            .flatten()
            .map(|info| ResolvedLink {
                url: link.target.clone(),
                title: info.title.unwrap_or_else(|| link.name.clone()),
            });
    }
}

/// How a URL is written where the frontmatter links are published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkUrlStyle {
    /// Site-absolute (`/customers/x/`): `site.json`, server pages, the API.
    Absolute,
    /// Relative to a static page `depth` folders below the root (`../x/`).
    RelativeToDepth(usize),
}

/// One link as templates, `site.json` and the info panel see it:
/// `{url, title, raw, text, resolved}` plus `label` and `anchor` when set.
///
/// `url` is the target even when nothing is there (as a broken body link is
/// still a link), so `resolved` says whether it exists; `title` is the
/// target's name when it does, else the value's own (`Customer X`, the
/// file name).
pub fn link_json(link: &FrontmatterLink, style: LinkUrlStyle) -> Value {
    let href = format!(
        "{}{}",
        link.target,
        link.anchor.as_deref().unwrap_or_default()
    );
    let url = match style {
        LinkUrlStyle::RelativeToDepth(depth) if link.internal => make_relative_url(&href, depth),
        _ => href,
    };
    let mut entry = Map::new();
    entry.insert("url".into(), Value::String(url));
    entry.insert("title".into(), Value::String(link.title().to_string()));
    entry.insert("text".into(), Value::String(link.text().to_string()));
    entry.insert("raw".into(), Value::String(link.raw.clone()));
    entry.insert("resolved".into(), Value::Bool(link.resolved.is_some()));
    if let Some(label) = &link.label {
        entry.insert("label".into(), Value::String(label.clone()));
    }
    if let Some(anchor) = &link.anchor {
        entry.insert("anchor".into(), Value::String(anchor.clone()));
    }
    Value::Object(entry)
}

/// `{key: entry}` for scalar values and `{key: [entries]}` for list values
/// (only the elements that are links, in list order) — the shape of the
/// `frontmatter_links` template variable and of each `site.json` entry's
/// `frontmatter_links`.
pub fn links_by_key(links: &[FrontmatterLink], style: LinkUrlStyle) -> Map<String, Value> {
    let mut out = Map::new();
    for link in links {
        let entry = link_json(link, style);
        if link.position.is_some() {
            match out
                .entry(link.key.clone())
                .or_insert_with(|| Value::Array(Vec::new()))
            {
                Value::Array(items) => items.push(entry),
                other => *other = Value::Array(vec![entry]),
            }
        } else {
            out.insert(link.key.clone(), entry);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use yaml_rust2::YamlLoader;

    fn yaml(text: &str) -> Yaml {
        YamlLoader::load_from_str(text).unwrap().remove(0)
    }

    fn page(url: &str, is_index: bool) -> LinkTransformConfig {
        LinkTransformConfig {
            markdown_extensions: vec!["md".to_string()],
            index_file: "index.md".to_string(),
            is_index_file: is_index,
            url_depth: None,
            current_page_url: url.to_string(),
            directory_url_probe: None,
            outside_links: None,
        }
    }

    fn found(text: &str) -> Vec<(String, Option<usize>, FrontmatterLinkForm, String)> {
        find_frontmatter_links(&yaml(text))
            .into_iter()
            .map(|s| (s.key, s.position, s.form, s.destination))
            .collect()
    }

    #[test]
    fn whole_value_wikilinks_and_paths_are_links() {
        use FrontmatterLinkForm::*;
        let links = found(concat!(
            "customer: \"[[Customer X]]\"\n",
            "owner: \"[[People/Ann|Ann B.]]\"\n",
            "contract: /customers/contracts/MSA (signed).pdf\n",
            "sibling: ./other.md\n",
            "up: ../up.pdf\n",
            "contracts:\n  - \"[[Customer X MSA 2026]]\"\n  - plain text\n  - /a/b.pdf\n",
        ));
        assert_eq!(
            links,
            vec![
                ("customer".into(), None, Wikilink, "Customer X".into()),
                ("owner".into(), None, Wikilink, "People/Ann".into()),
                (
                    "contract".into(),
                    None,
                    Path,
                    "/customers/contracts/MSA (signed).pdf".into()
                ),
                ("sibling".into(), None, Path, "./other.md".into()),
                ("up".into(), None, Path, "../up.pdf".into()),
                (
                    "contracts".into(),
                    Some(0),
                    Wikilink,
                    "Customer X MSA 2026".into()
                ),
                ("contracts".into(), Some(2), Path, "/a/b.pdf".into()),
            ]
        );
    }

    #[test]
    fn label_is_kept_and_trimmed() {
        let links = find_frontmatter_links(&yaml("a: \"[[ Target | Label ]]\"\nb: \"[[T|]]\"\n"));
        assert_eq!(links[0].destination, "Target");
        assert_eq!(links[0].label.as_deref(), Some("Label"));
        assert_eq!(links[1].label, None);
    }

    #[test]
    fn ordinary_strings_are_not_links() {
        let links = found(concat!(
            "ratio: 1/2\n",
            "text: \"see [[X]] here\"\n",
            "two: \"[[A]] [[B]]\"\n",
            "proto: //example.com/x\n",
            "url: https://example.com/x\n",
            "root: /\n",
            "dots: ./\n",
            "empty: \"[[]]\"\n",
            "anchor_only: \"[[#top]]\"\n",
            "multi: |\n  /a/b\n  /c/d\n",
            "nested:\n  inner: \"[[X]]\"\n",
            "list_of_maps:\n  - to: \"[[X]]\"\n",
            "number: 5\n",
        ));
        assert!(links.is_empty(), "{links:?}");
    }

    #[test]
    fn reserved_and_private_keys_are_not_links() {
        let links = found(concat!(
            "template: /invoices/tpl/default.html\n",
            "Relationships:\n  manager: \"[[Boss]]\"\n",
            "emails: \"[[Not a link]]\"\n",
            "addresses:\n  - /home/x\n",
            "customer: \"[[Kept]]\"\n",
        ));
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].0, "customer");
    }

    #[test]
    fn non_mapping_frontmatter_has_no_links() {
        assert!(find_frontmatter_links(&yaml("- a\n- b\n")).is_empty());
        assert!(find_frontmatter_links(&Yaml::Null).is_empty());
    }

    #[test]
    fn outbound_links_carry_origin_and_resolve_like_body_links() {
        let sources = find_frontmatter_links(&yaml(concat!(
            "customer: \"[[Customer X]]\"\n",
            "contract: \"/customers/contracts/MSA.pdf\"\n",
            "note: ./sibling.md\n",
            "deep: \"../shared/x.pdf#page=2\"\n",
        )));
        let cfg = page("/invoices/inv-105/", false);
        let links =
            resolve_frontmatter_links(&sources, &cfg, &HashSet::new(), None, &AssumeTargetsExist);
        let targets: Vec<(&str, Option<&str>)> = links
            .iter()
            .map(|l| (l.target.as_str(), l.anchor.as_deref()))
            .collect();
        assert_eq!(
            targets,
            vec![
                // No index: the relative transform, like a body `[[Customer X]]`.
                ("/invoices/Customer X/", None),
                ("/customers/contracts/MSA.pdf", None),
                ("/invoices/sibling/", None),
                ("/shared/x.pdf", Some("#page=2")),
            ]
        );
        let outbound = frontmatter_outbound_links(&sources, &cfg, &HashSet::new(), None);
        assert!(outbound.iter().all(|l| l.internal));
        assert_eq!(
            outbound[0].origin,
            Some(LinkOrigin::Frontmatter {
                key: "customer".into()
            })
        );
    }

    #[test]
    fn wikilinks_use_the_global_index_like_body_wikilinks() {
        let index = WikilinkIndex::new();
        index.rebuild(&[crate::relationships::NoteRelInput {
            url: "/customers/customer-x/".into(),
            title: "Customer X".into(),
            stem: "customer-x".into(),
            aliases: vec![],
            is_index: false,
            relationships: vec![],
        }]);
        let sources = find_frontmatter_links(&yaml("customer: \"[[customer x|CX]]\"\n"));
        let links = resolve_frontmatter_links(
            &sources,
            &page("/invoices/inv/", false),
            &HashSet::new(),
            Some(&index),
            &AssumeTargetsExist,
        );
        assert_eq!(links[0].target, "/customers/customer-x/");
        assert_eq!(links[0].text(), "CX");
    }

    struct Known(&'static [(&'static str, &'static str)]);
    impl LinkTargetLookup for Known {
        fn lookup(&self, site_url: &str) -> Option<LinkTargetInfo> {
            self.0
                .iter()
                .find(|(url, _)| *url == site_url)
                .map(|(_, title)| LinkTargetInfo {
                    title: Some((*title).to_string()),
                })
        }
    }

    #[test]
    fn lookup_decides_resolved_and_title() {
        let sources = find_frontmatter_links(&yaml(
            "a: /x/known.pdf\nb: /x/missing.pdf\nc: \"[[known note]]\"\n",
        ));
        let links = resolve_frontmatter_links(
            &sources,
            &page("/", true),
            &HashSet::new(),
            None,
            &Known(&[
                ("/x/known.pdf", "known.pdf"),
                ("/known note/", "The Known Note"),
            ]),
        );
        assert_eq!(
            links[0].resolved,
            Some(ResolvedLink {
                url: "/x/known.pdf".into(),
                title: "known.pdf".into()
            })
        );
        assert_eq!(links[1].resolved, None);
        assert_eq!(links[2].resolved.as_ref().unwrap().title, "The Known Note");
    }

    #[test]
    fn links_by_key_groups_lists_and_relativizes() {
        let sources = find_frontmatter_links(&yaml(concat!(
            "customer: \"[[Customer X|CX]]\"\n",
            "contracts:\n  - /c/a.pdf\n  - text\n  - /c/b.pdf\n",
        )));
        let links = resolve_frontmatter_links(
            &sources,
            &page("/inv/one/", false),
            &HashSet::new(),
            None,
            &AssumeTargetsExist,
        );
        let abs = Value::Object(links_by_key(&links, LinkUrlStyle::Absolute));
        assert_eq!(
            abs,
            serde_json::json!({
                "customer": {
                    "url": "/inv/Customer X/", "title": "Customer X", "text": "CX",
                    "raw": "[[Customer X|CX]]", "label": "CX", "resolved": true
                },
                "contracts": [
                    {"url": "/c/a.pdf", "title": "a.pdf", "text": "a.pdf", "raw": "/c/a.pdf", "resolved": true},
                    {"url": "/c/b.pdf", "title": "b.pdf", "text": "b.pdf", "raw": "/c/b.pdf", "resolved": true},
                ]
            })
        );
        let rel = links_by_key(&links, LinkUrlStyle::RelativeToDepth(2));
        assert_eq!(rel["contracts"][0]["url"], "../../c/a.pdf");
    }

    #[test]
    fn unresolved_title_falls_back_to_the_value_name() {
        let sources = find_frontmatter_links(&yaml("c: \"[[Gone|G]]\"\np: /x/gone.pdf\n"));
        let links = resolve_frontmatter_links(
            &sources,
            &page("/", true),
            &HashSet::new(),
            None,
            &Known(&[]),
        );
        let json = links_by_key(&links, LinkUrlStyle::Absolute);
        assert_eq!(json["c"]["title"], "Gone");
        assert_eq!(json["c"]["text"], "G");
        assert_eq!(json["c"]["resolved"], false);
        assert_eq!(json["p"]["title"], "gone.pdf");
    }

    #[test]
    fn origin_round_trips_on_the_wire() {
        let link = OutboundLink {
            to: "/a/".into(),
            text: "A".into(),
            anchor: None,
            internal: true,
            origin: Some(LinkOrigin::Frontmatter { key: "k".into() }),
        };
        let json = serde_json::to_value(&link).unwrap();
        assert_eq!(json["kind"], "frontmatter");
        assert_eq!(json["key"], "k");
        assert_eq!(serde_json::from_value::<OutboundLink>(json).unwrap(), link);

        let body = OutboundLink {
            origin: None,
            ..link
        };
        let json = serde_json::to_value(&body).unwrap();
        assert!(json.get("kind").is_none(), "{json}");
        assert_eq!(serde_json::from_value::<OutboundLink>(json).unwrap(), body);
    }
}
