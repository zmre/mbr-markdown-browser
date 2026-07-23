//! Named, typed relationships between notes.
//!
//! Where [`crate::link_index`] tracks *untyped* content links (from markdown
//! `[text](url)`), this module tracks **named, typed relationships** declared in
//! YAML frontmatter, e.g. genealogy edges such as `parent`, `child`, `spouse`,
//! and `sibling`. Each edge can carry its own attributes (marriage/divorce
//! dates, place, notes...).
//!
//! # Model
//!
//! A relationship is a directed edge between two notes:
//!
//! ```text
//! subject --rel_type--> object       (read: "object is subject's rel_type")
//! ```
//!
//! - `from` names the **subject** (defaults to the current note when omitted).
//! - `to` names the **object** (defaults to the current note when omitted).
//! - `rel_type` labels the neighbour's role from the subject's viewpoint, so
//!   `type: parent, to: [[Sam]]` reads "Sam is my parent" and puts Sam under
//!   the current note's *Parents* group, while the reciprocal (derived) edge on
//!   Sam's note puts the current note under Sam's *Children* group.
//!
//! A [`RelationTypeRegistry`] (config-driven) classifies each `rel_type` as
//! `symmetric` (spouse, sibling) or one half of an `inverse` pair (parent ↔
//! child), so an author declares each edge **once** and the reverse is derived
//! automatically — analogous to mbr's bidirectional backlink derivation.
//!
//! Unknown relation types are tolerated: they are tracked directed with no
//! relabelling. Unresolved endpoints never fail the build — the raw string is
//! kept for display and a warning is emitted.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};

use papaya::HashMap as ConcurrentHashMap;
use serde::{Deserialize, Serialize};
use yaml_rust2::Yaml;

use crate::config::RelationType;
use crate::link_index::resolve_relative_url;

/// Reserved edge keys that are not treated as free-form attributes.
const RESERVED_EDGE_KEYS: &[&str] = &["type", "to", "from", "label"];

/// The direction of a relationship edge from a note's viewpoint.
///
/// `Outgoing` means the note is the **subject** of the canonical edge;
/// `Incoming` means it is the **object**. For symmetric relations the direction
/// is stable but not semantically meaningful (both endpoints see the same
/// predicate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Outgoing,
    Incoming,
}

/// A relationship exactly as declared in a note's frontmatter, with endpoints
/// left unresolved.
///
/// Parsed by [`parse_relationships`] straight from the raw YAML — this is the
/// dedicated typed path that avoids the lossy generic frontmatter converter
/// (which drops non-string array elements).
#[derive(Debug, Clone, PartialEq)]
pub struct RawRelationship {
    /// The relation predicate (e.g. "parent", "spouse"). Required.
    pub rel_type: String,
    /// The `to` endpoint (object), if present. Omitted => current note.
    pub to: Option<String>,
    /// The `from` endpoint (subject), if present. Omitted => current note.
    pub from: Option<String>,
    /// Optional explicit label overriding the registry label.
    pub label: Option<String>,
    /// Free-form edge attributes (everything except the reserved keys),
    /// preserved verbatim as JSON. Order-stable via `BTreeMap`.
    pub attributes: BTreeMap<String, serde_json::Value>,
}

/// A relationship after endpoint resolution and viewpoint framing, ready for
/// JSON exposure and rendering in the info panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedRelationship {
    /// The base relation type as declared (e.g. "parent").
    pub rel_type: String,
    /// The viewpoint-relative predicate used for grouping/labelling (e.g.
    /// "child" when this note is the target of a declared `parent` edge).
    pub predicate: String,
    /// The resolved neighbour URL path (empty string when unresolved).
    pub neighbor: String,
    /// The neighbour's display title (or the raw endpoint text when unresolved).
    pub neighbor_title: String,
    /// The original raw endpoint string (e.g. "[[Mary Doe]]").
    pub neighbor_raw: String,
    /// Whether the neighbour endpoint resolved to a known note.
    pub resolved: bool,
    /// Direction of the edge from this note's viewpoint.
    pub direction: Direction,
    /// Optional explicit edge label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Free-form edge attributes, preserved verbatim.
    pub attributes: BTreeMap<String, serde_json::Value>,
    /// True when this entry was *not* declared on the note it is attached to
    /// (i.e. produced by inverse/symmetric derivation or declared by a third
    /// note about two other notes).
    pub derived: bool,
}

/// Classifies relation types (symmetric / inverse pair / directed) and supplies
/// display labels. Built from the configured [`RelationType`]s.
#[derive(Debug, Clone)]
pub struct RelationTypeRegistry {
    /// Keyed by lowercased relation-type name.
    by_name: BTreeMap<String, RelationType>,
}

impl RelationTypeRegistry {
    /// Builds a registry from the configured relation types.
    pub fn from_types(types: &[RelationType]) -> Self {
        let by_name = types
            .iter()
            .map(|t| (t.name.to_lowercase(), t.clone()))
            .collect();
        Self { by_name }
    }

    /// Looks up a relation type by (case-insensitive) name.
    pub fn get(&self, name: &str) -> Option<&RelationType> {
        self.by_name.get(&name.to_lowercase())
    }

    /// Returns true if the named relation type is symmetric (e.g. spouse).
    pub fn is_symmetric(&self, name: &str) -> bool {
        self.get(name).is_some_and(|t| t.symmetric)
    }

    /// Returns the inverse relation-type name for the given type, if any
    /// (e.g. `parent` -> `child`).
    pub fn inverse_of(&self, name: &str) -> Option<String> {
        self.get(name).and_then(|t| t.inverse.clone())
    }

    /// Returns the canonical (configured) casing for a relation-type name,
    /// falling back to the input trimmed when the type is unknown.
    fn canonical_name(&self, name: &str) -> String {
        self.get(name)
            .map(|t| t.name.clone())
            .unwrap_or_else(|| name.trim().to_string())
    }

    /// The predicate shown on the **subject** side of an edge (the note that
    /// asserts "neighbour is my rel_type"): the declared type, unchanged.
    fn predicate_subject(&self, rel_type: &str) -> String {
        self.canonical_name(rel_type)
    }

    /// The predicate shown on the **object** side of an edge (the derived
    /// counterpart): the inverse for an inverse pair, the same name for a
    /// symmetric or unknown/directed type.
    fn predicate_object(&self, rel_type: &str) -> String {
        let lower = rel_type.to_lowercase();
        if self.is_symmetric(&lower) {
            return self.canonical_name(rel_type);
        }
        match self.inverse_of(&lower) {
            Some(inv) => self.canonical_name(&inv),
            None => self.canonical_name(rel_type),
        }
    }

    /// Serialises the registry for `site.json` as an array of type descriptors
    /// with resolved (concrete) labels for the frontend.
    pub fn to_json(&self) -> serde_json::Value {
        let arr: Vec<serde_json::Value> = self
            .by_name
            .values()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "symmetric": t.symmetric,
                    "inverse": t.inverse,
                    "label": t.singular_label(),
                    "label_plural": t.plural_label(),
                })
            })
            .collect();
        serde_json::Value::Array(arr)
    }
}

/// Input for building the relationship index: one entry per markdown note.
#[derive(Debug, Clone)]
pub struct NoteRelInput {
    /// The note's site URL path (e.g. "/people/john/").
    pub url: String,
    /// The note's display title.
    pub title: String,
    /// The note's filename stem (without extension), for `[[stem]]` resolution.
    pub stem: String,
    /// Alternate names (e.g. maiden names) that also resolve to this note.
    pub aliases: Vec<String>,
    /// Whether the note is an index file (affects relative path resolution).
    pub is_index: bool,
    /// The raw relationships declared on the note.
    pub relationships: Vec<RawRelationship>,
}

/// A thread-safe index of resolved relationships, keyed by note URL path.
///
/// Built once after a scan (when all titles are known) and rebuilt on file
/// changes in server mode — modelled on [`crate::tag_index::TagIndex`].
pub struct RelationshipIndex {
    /// url_path -> resolved relationships (declared + derived), deduped.
    by_note: ConcurrentHashMap<String, Vec<ResolvedRelationship>>,
    /// The relation-type registry (types + labels).
    registry: RelationTypeRegistry,
}

impl RelationshipIndex {
    /// Creates an empty index with the given registry.
    pub fn new(registry: RelationTypeRegistry) -> Self {
        Self {
            by_note: ConcurrentHashMap::new(),
            registry,
        }
    }

    /// Creates an empty index from the configured relation types.
    pub fn from_relation_types(types: &[RelationType]) -> Self {
        Self::new(RelationTypeRegistry::from_types(types))
    }

    /// Returns the relation-type registry.
    pub fn registry(&self) -> &RelationTypeRegistry {
        &self.registry
    }

    /// Normalises a note URL to the canonical index key: exactly one leading
    /// slash, with the remainder (including any trailing slash) left untouched.
    ///
    /// Applied on both insert and lookup so callers may pass URLs with or
    /// without a leading slash and still resolve to the same entry.
    fn normalize_url_key(url: &str) -> String {
        format!("/{}", url.trim_start_matches('/'))
    }

    /// Rebuilds the index from the given notes.
    ///
    /// Resolves endpoints, attributes each edge to both endpoints, applies the
    /// registry for inverse/symmetric predicates and reverse materialisation,
    /// and dedupes reciprocal declarations.
    pub fn rebuild(&self, notes: &[NoteRelInput], markdown_extensions: &[String]) {
        let map = build_relationship_map(notes, &self.registry, markdown_extensions);
        let guard = self.by_note.pin();
        guard.clear();
        for (url, rels) in map {
            guard.insert(Self::normalize_url_key(&url), rels);
        }
    }

    /// Returns the resolved relationships for a note URL (empty when none).
    pub fn get(&self, url: &str) -> Vec<ResolvedRelationship> {
        self.by_note
            .pin()
            .get(&Self::normalize_url_key(url))
            .cloned()
            .unwrap_or_default()
    }

    /// Clears the index.
    pub fn clear(&self) {
        self.by_note.pin().clear();
    }

    /// Returns the number of notes with at least one relationship.
    pub fn len(&self) -> usize {
        self.by_note.pin().len()
    }

    /// Returns true when no note has any relationship.
    pub fn is_empty(&self) -> bool {
        self.by_note.pin().is_empty()
    }

    /// Injects relationship data into a serialised `site.json` value.
    ///
    /// Adds a top-level `relationship_types` array and a per-note
    /// `relationships` array to each entry in `markdown_files`.
    pub fn inject_into_site_json(&self, value: &mut serde_json::Value) {
        let Some(obj) = value.as_object_mut() else {
            return;
        };
        obj.insert("relationship_types".to_string(), self.registry.to_json());

        if let Some(files) = obj.get_mut("markdown_files").and_then(|v| v.as_array_mut()) {
            // Pin the concurrent map once for the whole loop and look up via the
            // shared guard, serialising each note's relationships by reference
            // rather than re-pinning and cloning per entry.
            let guard = self.by_note.pin();
            for entry in files.iter_mut() {
                let url = entry
                    .get("url_path")
                    .and_then(|v| v.as_str())
                    .map(Self::normalize_url_key);
                if let Some(url) = url {
                    let value = match guard.get(&url) {
                        Some(rels) => serde_json::to_value(rels)
                            .unwrap_or_else(|_| serde_json::Value::Array(vec![])),
                        None => serde_json::Value::Array(vec![]),
                    };
                    if let Some(entry_obj) = entry.as_object_mut() {
                        entry_obj.insert("relationships".to_string(), value);
                    }
                }
            }
        }
    }
}

/// Parses the `relationships:` array-of-objects faithfully from the raw YAML.
///
/// This is the dedicated typed path: the generic frontmatter converter drops
/// non-string array elements, so relationships must be read straight from the
/// `yaml_rust2::Yaml` value. Malformed or empty entries are tolerated (skipped
/// with a warning); the endpoint strings are kept unresolved.
pub fn parse_relationships(yaml: &Yaml) -> Vec<RawRelationship> {
    let Some(hash) = yaml.as_hash() else {
        return Vec::new();
    };
    let rel_key = Yaml::String("relationships".to_string());
    let items = match hash.get(&rel_key) {
        Some(Yaml::Array(items)) => items,
        Some(_) => {
            tracing::warn!("`relationships` frontmatter is not an array; ignoring");
            return Vec::new();
        }
        None => return Vec::new(),
    };

    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let Some(item_hash) = item.as_hash() else {
            tracing::warn!("skipping relationship entry that is not a mapping");
            continue;
        };

        let rel_type = item_hash
            .get(&Yaml::String("type".to_string()))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string());
        let Some(rel_type) = rel_type.filter(|s| !s.is_empty()) else {
            tracing::warn!("skipping relationship entry with missing/empty `type`");
            continue;
        };

        let str_field = |name: &str| -> Option<String> {
            item_hash
                .get(&Yaml::String(name.to_string()))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };

        let to = str_field("to");
        let from = str_field("from");
        let label = str_field("label");

        let mut attributes = BTreeMap::new();
        for (k, v) in item_hash.iter() {
            if let Some(key) = k.as_str()
                && !RESERVED_EDGE_KEYS.contains(&key)
            {
                attributes.insert(key.to_string(), yaml_to_json(v));
            }
        }

        out.push(RawRelationship {
            rel_type,
            to,
            from,
            label,
            attributes,
        });
    }
    out
}

/// Converts a `yaml_rust2::Yaml` value to a `serde_json::Value`, preserving
/// structure (used for free-form edge attributes).
fn yaml_to_json(y: &Yaml) -> serde_json::Value {
    use serde_json::Value;
    match y {
        Yaml::String(s) => Value::String(s.clone()),
        Yaml::Integer(i) => Value::Number((*i).into()),
        Yaml::Real(r) => serde_json::Number::from_f64(r.parse::<f64>().unwrap_or(f64::NAN))
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(r.clone())),
        Yaml::Boolean(b) => Value::Bool(*b),
        Yaml::Array(items) => Value::Array(items.iter().map(yaml_to_json).collect()),
        Yaml::Hash(h) => {
            let mut map = serde_json::Map::with_capacity(h.len());
            for (k, v) in h.iter() {
                let key = k
                    .as_str()
                    .map(|s| s.to_string())
                    .or_else(|| k.as_i64().map(|i| i.to_string()))
                    .unwrap_or_default();
                map.insert(key, yaml_to_json(v));
            }
            Value::Object(map)
        }
        Yaml::Null | Yaml::BadValue | Yaml::Alias(_) => Value::Null,
    }
}

/// A resolved endpoint of a relationship edge.
#[derive(Debug, Clone)]
struct ResolvedEndpoint {
    /// Site URL path when resolved, else empty.
    url: String,
    /// Display title (resolved title, or the raw name when unresolved).
    title: String,
    /// Original raw endpoint text.
    raw: String,
    /// Whether the endpoint resolved to a known note.
    resolved: bool,
}

impl ResolvedEndpoint {
    /// Builds a resolved endpoint pointing at a known note.
    fn resolved(url: String, title: String, raw: &str) -> Self {
        Self {
            url,
            title,
            raw: raw.to_string(),
            resolved: true,
        }
    }

    /// Builds an unresolved endpoint that keeps its raw text for display.
    fn unresolved(title: String, raw: &str) -> Self {
        Self {
            url: String::new(),
            title,
            raw: raw.to_string(),
            resolved: false,
        }
    }

    /// Identity used for dedup/matching: the URL when resolved, otherwise a
    /// `~`-prefixed sentinel derived from the (normalised) display name so
    /// distinct unresolved endpoints don't collide with real notes.
    fn key(&self) -> Cow<'_, str> {
        if self.resolved {
            Cow::Borrowed(&self.url)
        } else {
            Cow::Owned(format!("~{}", normalize_name(&self.title)))
        }
    }
}

/// Name index used to resolve `[[Name]]`/stem endpoints to note URLs.
struct NameIndex {
    by_title: HashMap<String, String>,
    by_alias: HashMap<String, String>,
    by_stem: HashMap<String, String>,
    urls: HashSet<String>,
    title_by_url: HashMap<String, String>,
}

impl NameIndex {
    /// Builds the name index from notes. Ambiguous names resolve
    /// deterministically to the note with the lexicographically smallest URL
    /// (notes are visited in sorted URL order and the first insertion wins).
    fn build(notes: &[&NoteRelInput]) -> Self {
        let mut by_title = HashMap::new();
        let mut by_alias = HashMap::new();
        let mut by_stem = HashMap::new();
        let mut urls = HashSet::new();
        let mut title_by_url = HashMap::new();
        for note in notes {
            urls.insert(note.url.clone());
            title_by_url.insert(note.url.clone(), note.title.clone());
            by_title
                .entry(normalize_name(&note.title))
                .or_insert_with(|| note.url.clone());
            for alias in &note.aliases {
                by_alias
                    .entry(normalize_name(alias))
                    .or_insert_with(|| note.url.clone());
            }
            by_stem
                .entry(normalize_name(&note.stem))
                .or_insert_with(|| note.url.clone());
        }
        Self {
            by_title,
            by_alias,
            by_stem,
            urls,
            title_by_url,
        }
    }

    fn resolve(
        &self,
        raw: &str,
        source_url: &str,
        source_is_index: bool,
        markdown_extensions: &[String],
    ) -> ResolvedEndpoint {
        let trimmed = raw.trim();

        // 1. Wikilink `[[Name]]` (supports `[[Target|Alias]]`).
        if let Some(inner) = strip_wikilink(trimmed) {
            let target = inner.split('|').next().unwrap_or(inner).trim();
            return self.resolve_by_name(target, raw);
        }

        // 2. Path-like endpoints (relative or absolute file paths).
        if looks_like_path(trimmed, markdown_extensions) {
            let url = path_to_url(trimmed, source_url, source_is_index, markdown_extensions);
            if self.urls.contains(&url) {
                let title = self
                    .title_by_url
                    .get(&url)
                    .cloned()
                    .unwrap_or_else(|| url.clone());
                return ResolvedEndpoint::resolved(url, title, raw);
            }
            tracing::warn!("unresolved relationship endpoint path: {raw}");
            return ResolvedEndpoint::unresolved(trimmed.to_string(), raw);
        }

        // 3. Bare name -> title then filename stem.
        self.resolve_by_name(trimmed, raw)
    }

    fn resolve_by_name(&self, name: &str, raw: &str) -> ResolvedEndpoint {
        let key = normalize_name(name);
        // Resolution order: title -> alias -> filename stem.
        if let Some(url) = self
            .by_title
            .get(&key)
            .or_else(|| self.by_alias.get(&key))
            .or_else(|| self.by_stem.get(&key))
        {
            let title = self
                .title_by_url
                .get(url)
                .cloned()
                .unwrap_or_else(|| name.to_string());
            ResolvedEndpoint::resolved(url.clone(), title, raw)
        } else {
            tracing::warn!("unresolved relationship endpoint: {raw}");
            ResolvedEndpoint::unresolved(name.to_string(), raw)
        }
    }
}

/// An aggregated (deduped) logical edge.
struct AggregatedEdge {
    rel_type: String,
    subject: ResolvedEndpoint,
    object: ResolvedEndpoint,
    label: Option<String>,
    attributes: BTreeMap<String, serde_json::Value>,
    /// URLs of notes that declared this edge (for the `derived` flag).
    declarers: HashSet<String>,
}

/// Builds the full relationship map (url -> resolved relationships) from notes.
///
/// Modelled on the build-mode inbound-link inversion: each declared edge is
/// attributed to both endpoints, with the reverse materialised via the
/// registry. Reciprocal declarations are deduped by a canonical edge key.
pub fn build_relationship_map(
    notes: &[NoteRelInput],
    registry: &RelationTypeRegistry,
    markdown_extensions: &[String],
) -> BTreeMap<String, Vec<ResolvedRelationship>> {
    // Sort notes by URL for deterministic ambiguous-name resolution and stable
    // canonical-edge orientation.
    let mut sorted: Vec<&NoteRelInput> = notes.iter().collect();
    sorted.sort_by(|a, b| a.url.cmp(&b.url));

    let name_index = NameIndex::build(&sorted);

    let mut edges: BTreeMap<(String, String, String), AggregatedEdge> = BTreeMap::new();

    for note in &sorted {
        for rel in &note.relationships {
            let rel_type = rel.rel_type.trim();
            if rel_type.is_empty() {
                continue;
            }
            if rel.from.is_none() && rel.to.is_none() {
                tracing::warn!(
                    "relationship on {} has neither `to` nor `from`; skipping",
                    note.url
                );
                continue;
            }

            let self_ep =
                || ResolvedEndpoint::resolved(note.url.clone(), note.title.clone(), &note.title);

            let subject = match &rel.from {
                Some(s) => name_index.resolve(s, &note.url, note.is_index, markdown_extensions),
                None => self_ep(),
            };
            let object = match &rel.to {
                Some(s) => name_index.resolve(s, &note.url, note.is_index, markdown_extensions),
                None => self_ep(),
            };

            let subject_key = subject.key();
            let object_key = object.key();
            if subject_key == object_key {
                tracing::warn!(
                    "self-referential relationship on {} ({rel_type}); skipping",
                    note.url
                );
                continue;
            }

            let key = canonical_key(registry, rel_type, &subject_key, &object_key);
            let edge = edges.entry(key).or_insert_with(|| AggregatedEdge {
                rel_type: rel_type.to_string(),
                subject: subject.clone(),
                object: object.clone(),
                label: rel.label.clone(),
                attributes: rel.attributes.clone(),
                declarers: HashSet::new(),
            });
            edge.declarers.insert(note.url.clone());
            // Merge attributes (first declaration wins on key conflict).
            for (k, v) in &rel.attributes {
                edge.attributes
                    .entry(k.clone())
                    .or_insert_with(|| v.clone());
            }
            if edge.label.is_none() {
                edge.label = rel.label.clone();
            }
        }
    }

    // Materialise per-endpoint entries. Both sides share the same shape,
    // differing only in the anchor/neighbour roles, the viewpoint predicate,
    // and the direction, so one closure handles both.
    let mut result: BTreeMap<String, Vec<ResolvedRelationship>> = BTreeMap::new();
    for edge in edges.values() {
        let mut emit = |anchor: &ResolvedEndpoint,
                        neighbor: &ResolvedEndpoint,
                        predicate: String,
                        direction: Direction| {
            if !anchor.resolved {
                return;
            }
            let entry = ResolvedRelationship {
                rel_type: edge.rel_type.clone(),
                predicate,
                neighbor: neighbor.url.clone(),
                neighbor_title: neighbor.title.clone(),
                neighbor_raw: neighbor.raw.clone(),
                resolved: neighbor.resolved,
                direction,
                label: edge.label.clone(),
                attributes: edge.attributes.clone(),
                derived: !edge.declarers.contains(&anchor.url),
            };
            result.entry(anchor.url.clone()).or_default().push(entry);
        };
        emit(
            &edge.subject,
            &edge.object,
            registry.predicate_subject(&edge.rel_type),
            Direction::Outgoing,
        );
        emit(
            &edge.object,
            &edge.subject,
            registry.predicate_object(&edge.rel_type),
            Direction::Incoming,
        );
    }

    // Deterministic ordering + defensive dedupe within each note.
    for rels in result.values_mut() {
        rels.sort_by(|a, b| {
            a.predicate
                .cmp(&b.predicate)
                .then_with(|| a.neighbor_title.cmp(&b.neighbor_title))
                .then_with(|| a.neighbor.cmp(&b.neighbor))
                .then_with(|| a.neighbor_raw.cmp(&b.neighbor_raw))
        });
        rels.dedup_by(|a, b| {
            a.predicate == b.predicate
                && a.neighbor == b.neighbor
                && a.neighbor_raw == b.neighbor_raw
                && a.direction == b.direction
        });
    }

    result
}

/// Computes a canonical dedup key for an edge so reciprocal declarations
/// collapse to a single logical edge.
///
/// - Symmetric: endpoints sorted (unordered pair).
/// - Inverse pair: normalised to the lexicographically smaller member, swapping
///   endpoints when needed.
/// - Directed/unknown: `(rel_type, subject, object)` as-is.
fn canonical_key(
    registry: &RelationTypeRegistry,
    rel_type: &str,
    subject_id: &str,
    object_id: &str,
) -> (String, String, String) {
    let lower = rel_type.to_lowercase();

    if registry.is_symmetric(&lower) {
        let name = registry.canonical_name(&lower);
        let (a, b) = if subject_id <= object_id {
            (subject_id, object_id)
        } else {
            (object_id, subject_id)
        };
        return (name, a.to_string(), b.to_string());
    }

    if let Some(inv) = registry.inverse_of(&lower) {
        let inv_lower = inv.to_lowercase();
        // The canonical member is the lexicographically smaller of the pair;
        // when `rel_type` is the *inverse* of it, swap the endpoints so both
        // declarations of the pair collapse to one key.
        let lower_is_canon = lower <= inv_lower;
        let canon = if lower_is_canon { &lower } else { &inv_lower };
        let name = registry.canonical_name(canon);
        if lower_is_canon {
            (name, subject_id.to_string(), object_id.to_string())
        } else {
            (name, object_id.to_string(), subject_id.to_string())
        }
    } else {
        (lower, subject_id.to_string(), object_id.to_string())
    }
}

/// Normalises a name for case-insensitive matching: trimmed and lowercased.
///
/// Shared with [`crate::wikilink_index`] so body-wikilink resolution uses the
/// exact same title/alias/stem matching as typed relationships.
pub(crate) fn normalize_name(s: &str) -> String {
    s.trim().to_lowercase()
}

fn strip_wikilink(s: &str) -> Option<&str> {
    s.strip_prefix("[[").and_then(|r| r.strip_suffix("]]"))
}

fn looks_like_path(s: &str, markdown_extensions: &[String]) -> bool {
    s.starts_with('/')
        || s.starts_with('.')
        || s.contains('/')
        || has_markdown_extension(s, markdown_extensions)
}

fn has_markdown_extension(s: &str, markdown_extensions: &[String]) -> bool {
    s.rsplit_once('.')
        .is_some_and(|(_, ext)| crate::repo::is_markdown_extension(ext, markdown_extensions))
}

/// Converts a path endpoint to a site URL, stripping a markdown extension and
/// resolving relative paths against the source note's URL.
fn path_to_url(
    path: &str,
    source_url: &str,
    source_is_index: bool,
    markdown_extensions: &[String],
) -> String {
    // Drop any anchor fragment.
    let (path, _anchor) = crate::link_index::split_url_anchor(path);
    let relative = markdown_path_to_slash(&path, markdown_extensions);
    resolve_relative_url(source_url, &relative, source_is_index)
}

/// Turns `a/b.md` into `a/b/` (trailing-slash URL convention); leaves paths
/// without a markdown extension unchanged.
fn markdown_path_to_slash(path: &str, markdown_extensions: &[String]) -> String {
    if let Some((base, ext)) = path.rsplit_once('.')
        && crate::repo::is_markdown_extension(ext, markdown_extensions)
    {
        return format!("{}/", base);
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use yaml_rust2::YamlLoader;

    fn genealogy_types() -> Vec<RelationType> {
        crate::config::default_relationship_types()
    }

    fn parse_yaml(s: &str) -> Yaml {
        YamlLoader::load_from_str(s)
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
    }

    fn note(url: &str, title: &str, stem: &str, rels: Vec<RawRelationship>) -> NoteRelInput {
        NoteRelInput {
            url: url.to_string(),
            title: title.to_string(),
            stem: stem.to_string(),
            aliases: Vec::new(),
            is_index: false,
            relationships: rels,
        }
    }

    // ----- parse_relationships -----

    #[test]
    fn parse_array_of_objects() {
        let yaml = parse_yaml(
            "type: person\nrelationships:\n  - type: spouse\n    to: \"[[Mary Doe]]\"\n    married: 1925-06-01\n    divorced: 1940-03-14\n  - type: parent\n    to: \"[[Alice Doe]]\"\n",
        );
        let rels = parse_relationships(&yaml);
        assert_eq!(rels.len(), 2);
        assert_eq!(rels[0].rel_type, "spouse");
        assert_eq!(rels[0].to.as_deref(), Some("[[Mary Doe]]"));
        assert!(rels[0].from.is_none());
        assert_eq!(
            rels[0].attributes.get("married"),
            Some(&serde_json::Value::String("1925-06-01".to_string()))
        );
        assert_eq!(
            rels[0].attributes.get("divorced"),
            Some(&serde_json::Value::String("1940-03-14".to_string()))
        );
        assert_eq!(rels[1].rel_type, "parent");
        assert_eq!(rels[1].to.as_deref(), Some("[[Alice Doe]]"));
    }

    #[test]
    fn parse_from_endpoint() {
        let yaml = parse_yaml("relationships:\n  - type: child\n    from: \"[[Sam Doe]]\"\n");
        let rels = parse_relationships(&yaml);
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].from.as_deref(), Some("[[Sam Doe]]"));
        assert!(rels[0].to.is_none());
    }

    #[test]
    fn parse_both_present_edge() {
        let yaml = parse_yaml(
            "relationships:\n  - type: parent\n    from: \"[[A]]\"\n    to: \"[[B]]\"\n",
        );
        let rels = parse_relationships(&yaml);
        assert_eq!(rels[0].from.as_deref(), Some("[[A]]"));
        assert_eq!(rels[0].to.as_deref(), Some("[[B]]"));
    }

    #[test]
    fn parse_preserves_typed_attributes() {
        let yaml = parse_yaml(
            "relationships:\n  - type: spouse\n    to: \"[[X]]\"\n    since_year: 1925\n    happy: true\n    place: \"Denver, CO\"\n",
        );
        let rels = parse_relationships(&yaml);
        assert_eq!(
            rels[0].attributes.get("since_year"),
            Some(&serde_json::json!(1925))
        );
        assert_eq!(
            rels[0].attributes.get("happy"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(
            rels[0].attributes.get("place"),
            Some(&serde_json::Value::String("Denver, CO".to_string()))
        );
    }

    #[test]
    fn parse_tolerates_missing_type_and_malformed() {
        let yaml = parse_yaml(
            "relationships:\n  - to: \"[[X]]\"\n  - type: spouse\n    to: \"[[Y]]\"\n  - just a string\n",
        );
        let rels = parse_relationships(&yaml);
        // Only the well-formed spouse entry survives.
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].rel_type, "spouse");
    }

    #[test]
    fn parse_empty_when_no_relationships_key() {
        let yaml = parse_yaml("title: Just A Note\ntags:\n  - a\n  - b\n");
        assert!(parse_relationships(&yaml).is_empty());
    }

    #[test]
    fn parse_non_array_relationships_ignored() {
        let yaml = parse_yaml("relationships: not-an-array\n");
        assert!(parse_relationships(&yaml).is_empty());
    }

    // ----- registry -----

    #[test]
    fn registry_symmetric_and_inverse() {
        let reg = RelationTypeRegistry::from_types(&genealogy_types());
        assert!(reg.is_symmetric("spouse"));
        assert!(reg.is_symmetric("SPOUSE"));
        assert!(reg.is_symmetric("sibling"));
        assert!(!reg.is_symmetric("parent"));
        assert_eq!(reg.inverse_of("parent").as_deref(), Some("child"));
        assert_eq!(reg.inverse_of("child").as_deref(), Some("parent"));
        assert_eq!(reg.predicate_object("parent"), "child");
        assert_eq!(reg.predicate_object("child"), "parent");
        assert_eq!(reg.predicate_object("spouse"), "spouse");
        assert_eq!(reg.predicate_subject("parent"), "parent");
    }

    #[test]
    fn registry_to_json_has_labels() {
        let reg = RelationTypeRegistry::from_types(&genealogy_types());
        let json = reg.to_json();
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 4);
        let child = arr.iter().find(|t| t["name"] == "child").unwrap();
        assert_eq!(child["label_plural"], "Children");
        assert_eq!(child["inverse"], "parent");
    }

    // ----- endpoint resolution -----

    #[test]
    fn resolve_by_title_and_stem_case_insensitive() {
        let notes = vec![
            note(
                "/people/john/",
                "John Doe",
                "john",
                vec![RawRelationship {
                    rel_type: "spouse".into(),
                    to: Some("[[mary doe]]".into()),
                    from: None,
                    label: None,
                    attributes: BTreeMap::new(),
                }],
            ),
            note("/people/mary/", "Mary Doe", "mary-doe-file", vec![]),
        ];
        let reg = RelationTypeRegistry::from_types(&genealogy_types());
        let map = build_relationship_map(&notes, &reg, &["md".to_string()]);
        let john = map.get("/people/john/").unwrap();
        assert_eq!(john[0].neighbor, "/people/mary/");
        assert!(john[0].resolved);
        // Mary sees the reciprocal (symmetric) edge.
        let mary = map.get("/people/mary/").unwrap();
        assert_eq!(mary[0].neighbor, "/people/john/");
        assert_eq!(mary[0].predicate, "spouse");
    }

    #[test]
    fn resolve_by_filename_stem() {
        let notes = vec![
            note(
                "/a/",
                "Alpha",
                "alpha",
                vec![RawRelationship {
                    rel_type: "spouse".into(),
                    to: Some("[[beta-stem]]".into()),
                    from: None,
                    label: None,
                    attributes: BTreeMap::new(),
                }],
            ),
            note("/b/", "Beta Person", "beta-stem", vec![]),
        ];
        let reg = RelationTypeRegistry::from_types(&genealogy_types());
        let map = build_relationship_map(&notes, &reg, &["md".to_string()]);
        assert_eq!(map.get("/a/").unwrap()[0].neighbor, "/b/");
    }

    #[test]
    fn resolve_broken_endpoint_kept_raw() {
        let notes = vec![note(
            "/a/",
            "Alpha",
            "alpha",
            vec![RawRelationship {
                rel_type: "spouse".into(),
                to: Some("[[Ghost]]".into()),
                from: None,
                label: None,
                attributes: BTreeMap::new(),
            }],
        )];
        let reg = RelationTypeRegistry::from_types(&genealogy_types());
        let map = build_relationship_map(&notes, &reg, &["md".to_string()]);
        let a = map.get("/a/").unwrap();
        assert_eq!(a.len(), 1);
        assert!(!a[0].resolved);
        assert_eq!(a[0].neighbor, "");
        assert_eq!(a[0].neighbor_raw, "[[Ghost]]");
        assert_eq!(a[0].neighbor_title, "Ghost");
    }

    #[test]
    fn resolve_ambiguous_title_is_deterministic() {
        // Two notes share the title "Sam"; the smaller URL wins.
        let notes = vec![
            note(
                "/z/",
                "Zeb",
                "zeb",
                vec![RawRelationship {
                    rel_type: "spouse".into(),
                    to: Some("[[Sam]]".into()),
                    from: None,
                    label: None,
                    attributes: BTreeMap::new(),
                }],
            ),
            note("/people/a-sam/", "Sam", "a-sam", vec![]),
            note("/people/b-sam/", "Sam", "b-sam", vec![]),
        ];
        let reg = RelationTypeRegistry::from_types(&genealogy_types());
        let map = build_relationship_map(&notes, &reg, &["md".to_string()]);
        assert_eq!(map.get("/z/").unwrap()[0].neighbor, "/people/a-sam/");
    }

    #[test]
    fn resolve_path_endpoint() {
        let notes = vec![
            note(
                "/people/john/",
                "John",
                "john",
                vec![RawRelationship {
                    rel_type: "spouse".into(),
                    to: Some("mary.md".into()),
                    from: None,
                    label: None,
                    attributes: BTreeMap::new(),
                }],
            ),
            note("/people/mary/", "Mary", "mary", vec![]),
        ];
        let reg = RelationTypeRegistry::from_types(&genealogy_types());
        let map = build_relationship_map(&notes, &reg, &["md".to_string()]);
        assert_eq!(
            map.get("/people/john/").unwrap()[0].neighbor,
            "/people/mary/"
        );
    }

    #[test]
    fn resolve_by_alias() {
        // A references Mary by her maiden name; Mary's note lists it as an alias.
        let mut mary = note("/people/mary/", "Mary Smith", "mary", vec![]);
        mary.aliases = vec!["Mary Doe".into()];
        let notes = vec![
            note(
                "/z/",
                "Zeb",
                "zeb",
                vec![RawRelationship {
                    rel_type: "spouse".into(),
                    to: Some("[[Mary Doe]]".into()),
                    from: None,
                    label: None,
                    attributes: BTreeMap::new(),
                }],
            ),
            mary,
        ];
        let reg = RelationTypeRegistry::from_types(&genealogy_types());
        let map = build_relationship_map(&notes, &reg, &["md".to_string()]);
        assert_eq!(map.get("/z/").unwrap()[0].neighbor, "/people/mary/");
    }

    #[test]
    fn title_beats_alias() {
        // "Sam" is one note's title and another note's alias; the title wins
        // (resolution order is title -> alias -> stem).
        let mut alias_holder = note("/people/aka/", "Samuel", "samuel", vec![]);
        alias_holder.aliases = vec!["Sam".into()];
        let notes = vec![
            note(
                "/z/",
                "Zeb",
                "zeb",
                vec![RawRelationship {
                    rel_type: "spouse".into(),
                    to: Some("[[Sam]]".into()),
                    from: None,
                    label: None,
                    attributes: BTreeMap::new(),
                }],
            ),
            note("/people/sam/", "Sam", "sam", vec![]),
            alias_holder,
        ];
        let reg = RelationTypeRegistry::from_types(&genealogy_types());
        let map = build_relationship_map(&notes, &reg, &["md".to_string()]);
        assert_eq!(map.get("/z/").unwrap()[0].neighbor, "/people/sam/");
    }

    #[test]
    fn alias_case_insensitive() {
        // A differently-cased alias still resolves a differently-cased endpoint.
        let mut mary = note("/people/mary/", "Mary Smith", "mary", vec![]);
        mary.aliases = vec!["Mary DOE".into()];
        let notes = vec![
            note(
                "/z/",
                "Zeb",
                "zeb",
                vec![RawRelationship {
                    rel_type: "spouse".into(),
                    to: Some("[[mary doe]]".into()),
                    from: None,
                    label: None,
                    attributes: BTreeMap::new(),
                }],
            ),
            mary,
        ];
        let reg = RelationTypeRegistry::from_types(&genealogy_types());
        let map = build_relationship_map(&notes, &reg, &["md".to_string()]);
        assert_eq!(map.get("/z/").unwrap()[0].neighbor, "/people/mary/");
    }

    // ----- inverse / symmetric derivation -----

    #[test]
    fn inverse_parent_child_derivation() {
        // John declares "Alice is my child" -> Alice sees John as parent.
        let notes = vec![
            note(
                "/john/",
                "John",
                "john",
                vec![RawRelationship {
                    rel_type: "child".into(),
                    to: Some("[[Alice]]".into()),
                    from: None,
                    label: None,
                    attributes: BTreeMap::new(),
                }],
            ),
            note("/alice/", "Alice", "alice", vec![]),
        ];
        let reg = RelationTypeRegistry::from_types(&genealogy_types());
        let map = build_relationship_map(&notes, &reg, &["md".to_string()]);

        let john = map.get("/john/").unwrap();
        assert_eq!(john[0].predicate, "child");
        assert_eq!(john[0].neighbor, "/alice/");
        assert!(!john[0].derived);

        let alice = map.get("/alice/").unwrap();
        assert_eq!(alice[0].predicate, "parent");
        assert_eq!(alice[0].neighbor, "/john/");
        assert!(alice[0].derived);
    }

    #[test]
    fn symmetric_spouse_appears_both_sides() {
        let notes = vec![
            note(
                "/john/",
                "John",
                "john",
                vec![RawRelationship {
                    rel_type: "spouse".into(),
                    to: Some("[[Mary]]".into()),
                    from: None,
                    label: None,
                    attributes: BTreeMap::new(),
                }],
            ),
            note("/mary/", "Mary", "mary", vec![]),
        ];
        let reg = RelationTypeRegistry::from_types(&genealogy_types());
        let map = build_relationship_map(&notes, &reg, &["md".to_string()]);
        assert_eq!(map.get("/john/").unwrap()[0].predicate, "spouse");
        assert_eq!(map.get("/mary/").unwrap()[0].predicate, "spouse");
    }

    #[test]
    fn reciprocal_declarations_deduped() {
        // John: "Alice is my child"; Alice: "John is my parent". Same edge.
        let notes = vec![
            note(
                "/john/",
                "John",
                "john",
                vec![RawRelationship {
                    rel_type: "child".into(),
                    to: Some("[[Alice]]".into()),
                    from: None,
                    label: None,
                    attributes: BTreeMap::new(),
                }],
            ),
            note(
                "/alice/",
                "Alice",
                "alice",
                vec![RawRelationship {
                    rel_type: "parent".into(),
                    to: Some("[[John]]".into()),
                    from: None,
                    label: None,
                    attributes: BTreeMap::new(),
                }],
            ),
        ];
        let reg = RelationTypeRegistry::from_types(&genealogy_types());
        let map = build_relationship_map(&notes, &reg, &["md".to_string()]);
        // Exactly one edge on each side (not duplicated).
        assert_eq!(map.get("/john/").unwrap().len(), 1);
        assert_eq!(map.get("/alice/").unwrap().len(), 1);
        // Both declared it, so neither side is "derived".
        assert!(!map.get("/john/").unwrap()[0].derived);
        assert!(!map.get("/alice/").unwrap()[0].derived);
    }

    #[test]
    fn both_present_edge_between_other_notes() {
        // C declares an edge between A and B; both get derived entries.
        let notes = vec![
            note(
                "/c/",
                "C",
                "c",
                vec![RawRelationship {
                    rel_type: "spouse".into(),
                    from: Some("[[A]]".into()),
                    to: Some("[[B]]".into()),
                    label: None,
                    attributes: BTreeMap::new(),
                }],
            ),
            note("/a/", "A", "a", vec![]),
            note("/b/", "B", "b", vec![]),
        ];
        let reg = RelationTypeRegistry::from_types(&genealogy_types());
        let map = build_relationship_map(&notes, &reg, &["md".to_string()]);
        assert!(!map.contains_key("/c/"));
        assert!(map.get("/a/").unwrap()[0].derived);
        assert!(map.get("/b/").unwrap()[0].derived);
    }

    #[test]
    fn unknown_type_directed_no_relabel() {
        let notes = vec![
            note(
                "/john/",
                "John",
                "john",
                vec![RawRelationship {
                    rel_type: "mentor".into(),
                    to: Some("[[Alice]]".into()),
                    from: None,
                    label: None,
                    attributes: BTreeMap::new(),
                }],
            ),
            note("/alice/", "Alice", "alice", vec![]),
        ];
        let reg = RelationTypeRegistry::from_types(&genealogy_types());
        let map = build_relationship_map(&notes, &reg, &["md".to_string()]);
        assert_eq!(map.get("/john/").unwrap()[0].predicate, "mentor");
        // Counterpart still visible, no relabel.
        assert_eq!(map.get("/alice/").unwrap()[0].predicate, "mentor");
        assert_eq!(
            map.get("/alice/").unwrap()[0].direction,
            Direction::Incoming
        );
    }

    #[test]
    fn neither_endpoint_is_skipped() {
        let notes = vec![note(
            "/john/",
            "John",
            "john",
            vec![RawRelationship {
                rel_type: "spouse".into(),
                to: None,
                from: None,
                label: None,
                attributes: BTreeMap::new(),
            }],
        )];
        let reg = RelationTypeRegistry::from_types(&genealogy_types());
        let map = build_relationship_map(&notes, &reg, &["md".to_string()]);
        assert!(map.is_empty());
    }

    #[test]
    fn index_rebuild_and_get() {
        let index = RelationshipIndex::from_relation_types(&genealogy_types());
        assert!(index.is_empty());
        let notes = vec![
            note(
                "/john/",
                "John",
                "john",
                vec![RawRelationship {
                    rel_type: "spouse".into(),
                    to: Some("[[Mary]]".into()),
                    from: None,
                    label: None,
                    attributes: BTreeMap::new(),
                }],
            ),
            note("/mary/", "Mary", "mary", vec![]),
        ];
        index.rebuild(&notes, &["md".to_string()]);
        assert_eq!(index.get("/john/").len(), 1);
        assert_eq!(index.get("/mary/").len(), 1);
        // Rebuild replaces prior contents.
        index.rebuild(&[], &["md".to_string()]);
        assert!(index.is_empty());
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// A generated edge: `declarer` note declares `type` about the `target` note.
    #[derive(Debug, Clone)]
    struct GenEdge {
        declarer: usize,
        target: usize,
        symmetric: bool,
    }

    fn edges_strategy(n: usize) -> impl Strategy<Value = Vec<GenEdge>> {
        prop::collection::vec(
            (0..n, 0..n, any::<bool>()).prop_map(|(declarer, target, symmetric)| GenEdge {
                declarer,
                target,
                symmetric,
            }),
            0..12,
        )
    }

    fn url(i: usize) -> String {
        format!("/p{i}/")
    }

    proptest! {
        /// Inverse invariant: declaring `type: parent, to: [[Pj]]` on note Pi
        /// makes Pi see Pj as "parent" and (by inverse derivation) Pj see Pi as
        /// "child". Symmetric closure: `spouse` appears on both endpoints.
        #[test]
        fn inverse_and_symmetric_closure(edges in edges_strategy(6)) {
            let n = 6;
            let reg = RelationTypeRegistry::from_types(&crate::config::default_relationship_types());

            // Build per-note relationship lists (skip self-loops).
            let mut per_note: Vec<Vec<RawRelationship>> = vec![Vec::new(); n];
            for e in &edges {
                if e.declarer == e.target {
                    continue;
                }
                let rel_type = if e.symmetric { "spouse" } else { "parent" };
                per_note[e.declarer].push(RawRelationship {
                    rel_type: rel_type.to_string(),
                    to: Some(format!("[[P{}]]", e.target)),
                    from: None,
                    label: None,
                    attributes: BTreeMap::new(),
                });
            }

            let notes: Vec<NoteRelInput> = (0..n)
                .map(|i| NoteRelInput {
                    url: url(i),
                    title: format!("P{i}"),
                    stem: format!("p{i}"),
                    aliases: Vec::new(),
                    is_index: false,
                    relationships: per_note[i].clone(),
                })
                .collect();

            let map = build_relationship_map(&notes, &reg, &["md".to_string()]);

            for e in &edges {
                if e.declarer == e.target {
                    continue;
                }
                let a = url(e.declarer);
                let b = url(e.target);
                let a_rels = map.get(&a).cloned().unwrap_or_default();
                let b_rels = map.get(&b).cloned().unwrap_or_default();

                if e.symmetric {
                    // spouse: both endpoints see each other under "spouse".
                    prop_assert!(a_rels
                        .iter()
                        .any(|r| r.predicate == "spouse" && r.neighbor == b));
                    prop_assert!(b_rels
                        .iter()
                        .any(|r| r.predicate == "spouse" && r.neighbor == a));
                } else {
                    // Declarer sees target as "parent"; target sees declarer as
                    // the inverse "child".
                    prop_assert!(a_rels
                        .iter()
                        .any(|r| r.predicate == "parent" && r.neighbor == b));
                    prop_assert!(b_rels
                        .iter()
                        .any(|r| r.predicate == "child" && r.neighbor == a));
                }
            }
        }
    }
}
