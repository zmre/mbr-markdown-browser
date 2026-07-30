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
//! automatically — analogous to mbr's bidirectional backlink derivation. Only
//! one half of an inverse pair need be configured: the registry auto-registers
//! the reciprocal (see [`RelationTypeRegistry::from_types`]).
//!
//! Unknown relation types are tolerated: they are tracked directed with no
//! relabelling. Unresolved endpoints never fail the build — the raw string is
//! kept for display and a warning is emitted.

use std::borrow::Cow;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

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

/// A cycle detected over the **hierarchical** relationship edges.
///
/// Only inverse-pair types (`parent` ↔ `child`, `employer` ↔ `employee`)
/// participate: they describe a hierarchy, so a cycle is data that cannot be
/// true — and it is what makes `family-chart`'s `d3.hierarchy()` allocate
/// forever on a person page. Symmetric types (`spouse`, `sibling`) and plain
/// directed types cycle legitimately and are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationshipCycle {
    /// Note URLs forming the cycle, in traversal order: each entry is the
    /// [`Self::rel_type`] of the entry before it, and the last closes back onto
    /// the first.
    pub members: Vec<String>,
    /// The canonical relation type whose edges close the cycle (e.g. "child").
    pub rel_type: String,
}

/// A relationship endpoint that resolved through a name owned by more than one
/// note.
///
/// Resolution itself is unchanged (first-wins, lexicographically-smallest URL);
/// this only records that the choice was arbitrary so the author can be told.
/// In a genealogy repository a John Doe Sr/Jr pair makes this the likeliest way
/// to accidentally close a [`RelationshipCycle`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmbiguousEndpoint {
    /// The endpoint exactly as authored, e.g. "[[John Doe]]".
    pub raw: String,
    /// The note URL mbr resolved it to.
    pub resolved_to: String,
    /// The other notes sharing that name.
    pub candidates: Vec<String>,
}

/// Maximum number of individual ambiguous-name warnings logged per index build.
///
/// The user repository that motivated this is 585 genealogy notes where
/// namesakes are the norm, so the ambiguous-name count can run to dozens. One
/// log line each would scroll every other startup warning off the screen.
const AMBIGUOUS_WARN_CAP: usize = 20;

/// One ambiguous name, ready for logging.
pub(crate) struct AmbiguousNameReport<'a> {
    /// The (normalised) name several notes answer to.
    pub name: &'a str,
    /// The note URL that wins resolution.
    pub winner: &'a str,
    /// The other note URLs sharing the name.
    pub others: Vec<&'a str>,
}

/// Logs ambiguous-name warnings, capped at [`AMBIGUOUS_WARN_CAP`] individual
/// lines plus one summary line for the remainder.
///
/// `kind` names the thing that was ambiguous ("relationship endpoint",
/// "wikilink") and is interpolated into the message. Shared with
/// [`crate::wikilink_index`], which has its own index but the identical
/// first-wins behaviour to report on. The per-page `errors.json` payload is
/// deliberately *not* capped — a reader only ever sees their own page's
/// problems, so there is nothing to bury there.
pub(crate) fn warn_ambiguous_names(kind: &str, reports: &[AmbiguousNameReport<'_>]) {
    for report in reports.iter().take(AMBIGUOUS_WARN_CAP) {
        tracing::warn!(
            "ambiguous {kind} name `{}`: resolved to {}; also matched by {}. \
             mbr always picks the first — rename a note, give it a distinguishing \
             `aliases:` entry, or name the file explicitly to choose",
            report.name,
            report.winner,
            report.others.join(", "),
        );
    }
    let extra = reports.len().saturating_sub(AMBIGUOUS_WARN_CAP);
    if extra > 0 {
        tracing::warn!(
            "... and {extra} more ambiguous {kind} names; each affected page lists \
             its own in the page-problems panel"
        );
    }
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
    ///
    /// Configured types are taken verbatim and **always win**. For every
    /// non-symmetric type that names an `inverse`, the reciprocal half of the
    /// pair is auto-registered (with the inverse pointing back) when it is not
    /// itself configured — so declaring only
    /// `{ name = "employer", inverse = "employee" }` still relabels *both* sides
    /// of an edge and still collapses reciprocal declarations into one edge.
    ///
    /// When the reciprocal *is* configured but disagrees (`employer` claims
    /// `employee`, while `employee` claims `staff`), both are kept exactly as
    /// configured and a warning naming both is logged. A type that is at once
    /// `symmetric` and `inverse` keeps its symmetric meaning (matching
    /// [`Self::predicate_object`]) and logs a warning.
    ///
    /// A type naming *itself* as its `inverse` is self-contradictory and is
    /// coerced to symmetric — see [`coerce_self_inverse`].
    pub fn from_types(types: &[RelationType]) -> Self {
        // Repair self-contradictory declarations before anything reads them, so
        // every consumer sees one coherent definition: this registry,
        // `canonical_key`, cycle detection, and — via the `symmetric` flag in
        // `to_json` — the frontend's own relationship classifier.
        let types: Vec<RelationType> = types.iter().map(coerce_self_inverse).collect();
        let configured: BTreeMap<String, RelationType> = types
            .iter()
            .map(|t| (t.name.to_lowercase(), t.clone()))
            .collect();
        Self {
            by_name: derive_reciprocal_types(configured, &types),
        }
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
    ///
    /// The name is trimmed, and a blank `inverse` reads as no inverse at all —
    /// matching the auto-registration in [`Self::from_types`], which also
    /// ignores it. Without that filter `predicate_object` would relabel the
    /// reverse edge to the empty string.
    pub fn inverse_of(&self, name: &str) -> Option<String> {
        self.get(name)
            .and_then(|t| t.inverse.as_deref())
            .map(str::trim)
            .filter(|inverse| !inverse.is_empty())
            .map(str::to_string)
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

/// Repairs a type that names *itself* as its `inverse`, returning the symmetric
/// type its author meant. Any other type is returned unchanged.
///
/// `inverse` names the **other** half of a pair, so a self-inverse declaration is
/// self-contradictory. Both endpoints of such an edge derive the *same*
/// predicate, which is exactly what `symmetric = true` means — so "mutual" is
/// almost certainly the intent.
///
/// Coercing (rather than tolerating) matters because a nominal inverse pair whose
/// two halves are indistinguishable looks like a **2-cycle** to anything that
/// orients hierarchical edges by predicate. That includes the frontend's
/// `classifyRelationship`, which tests `symmetric` before `inverse`: left
/// uncoerced it would take the hierarchical branch, emit two dedup keys for one
/// logical edge, drop half of every such relationship, and tell the reader their
/// notes each claim to be the other's ancestor — blaming their data for a config
/// mistake. Fixing it here propagates automatically, because `symmetric` is
/// serialised into `relationship_types` in `site.json`.
fn coerce_self_inverse(t: &RelationType) -> RelationType {
    let is_self_inverse = t
        .inverse
        .as_deref()
        .is_some_and(|inverse| normalize_name(inverse) == normalize_name(&t.name));
    if !is_self_inverse {
        return t.clone();
    }
    tracing::warn!(
        "relation type `{}` names itself as its own `inverse`; a type cannot be \
         its own inverse (`inverse` names the *other* half of a pair) and both \
         ends of such an edge mean the same thing, so it is being treated as \
         `symmetric = true`. Declare `symmetric = true` and drop `inverse` to \
         silence this.",
        t.name,
    );
    RelationType {
        symmetric: true,
        inverse: None,
        ..t.clone()
    }
}

/// Builds the reciprocal half of an inverse pair, ready for auto-registration.
///
/// Returns `None` (with a warning) when the declaration cannot yield a usable
/// reciprocal: a blank `inverse`, or a type that is both `symmetric` and
/// `inverse` — symmetric wins there, matching
/// [`RelationTypeRegistry::predicate_object`] and [`canonical_key`].
fn reciprocal_type(t: &RelationType) -> Option<RelationType> {
    let inverse = t.inverse.as_deref()?.trim();
    if inverse.is_empty() {
        tracing::warn!(
            "relation type `{}` declares an empty `inverse`; ignoring it",
            t.name
        );
        return None;
    }
    if t.symmetric {
        tracing::warn!(
            "relation type `{}` is both `symmetric` and `inverse = \"{inverse}\"`; \
             ignoring the inverse (symmetric wins)",
            t.name
        );
        return None;
    }
    Some(RelationType {
        name: inverse.to_string(),
        symmetric: false,
        inverse: Some(t.name.trim().to_string()),
        label: None,
        label_plural: None,
    })
}

/// Completes half-declared inverse pairs by auto-registering the reciprocal of
/// every configured type that names an `inverse`.
///
/// Configured types are never overwritten; a reciprocal that already exists and
/// disagrees is left alone and logged. See [`RelationTypeRegistry::from_types`].
fn derive_reciprocal_types(
    configured: BTreeMap<String, RelationType>,
    types: &[RelationType],
) -> BTreeMap<String, RelationType> {
    types
        .iter()
        .filter_map(|t| reciprocal_type(t).map(|reciprocal| (t, reciprocal)))
        .fold(configured, |mut acc, (t, reciprocal)| {
            let key = reciprocal.name.to_lowercase();
            let was_configured = types.iter().any(|other| other.name.to_lowercase() == key);
            match acc.entry(key) {
                Entry::Vacant(slot) => {
                    slot.insert(reciprocal);
                }
                Entry::Occupied(slot) => {
                    let existing = slot.get();
                    let agrees = existing
                        .inverse
                        .as_deref()
                        .is_some_and(|i| normalize_name(i) == normalize_name(&t.name));
                    if !agrees {
                        let origin = if was_configured {
                            "configured"
                        } else {
                            "auto-derived"
                        };
                        tracing::warn!(
                            "relation type `{}` declares `inverse = \"{}\"`, but the {origin} \
                             type `{}` has `inverse = {}`; keeping both as-is — reverse edges \
                             for one of them will not be relabelled as expected",
                            t.name,
                            reciprocal.name,
                            existing.name,
                            existing.inverse.as_deref().unwrap_or("none"),
                        );
                    }
                }
            }
            acc
        })
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
    /// Member note url_path -> every hierarchical cycle it participates in.
    ///
    /// Keyed by *member* (not by declarer): a cycle is a property of the notes
    /// caught in it, and those are the notes whose data has to change.
    /// Recomputed by [`Self::rebuild`], so it always matches `by_note`.
    cycles_by_note: ConcurrentHashMap<String, Vec<RelationshipCycle>>,
    /// Declaring note url_path -> its own endpoints that resolved through a name
    /// shared by several notes. Keyed by *declarer*, because that note's
    /// frontmatter is what needs disambiguating.
    ambiguous_by_note: ConcurrentHashMap<String, Vec<AmbiguousEndpoint>>,
}

impl RelationshipIndex {
    /// Creates an empty index with the given registry.
    pub fn new(registry: RelationTypeRegistry) -> Self {
        Self {
            by_note: ConcurrentHashMap::new(),
            registry,
            cycles_by_note: ConcurrentHashMap::new(),
            ambiguous_by_note: ConcurrentHashMap::new(),
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
    /// Also detects and stores the data problems the build found (hierarchical
    /// cycles, ambiguous endpoint names) so they can be queried per note without
    /// re-walking the graph on the request path.
    pub fn rebuild(&self, notes: &[NoteRelInput], markdown_extensions: &[String]) {
        let built = build_relationships(notes, &self.registry, markdown_extensions);

        let guard = self.by_note.pin();
        guard.clear();
        for (url, rels) in built.by_note {
            guard.insert(Self::normalize_url_key(&url), rels);
        }

        // Invert cycles onto their members so a page lookup is a single hit.
        let mut cycles_by_member: BTreeMap<String, Vec<RelationshipCycle>> = BTreeMap::new();
        for cycle in &built.cycles {
            for member in &cycle.members {
                cycles_by_member
                    .entry(Self::normalize_url_key(member))
                    .or_default()
                    .push(cycle.clone());
            }
        }
        let cycles = self.cycles_by_note.pin();
        cycles.clear();
        for (url, member_cycles) in cycles_by_member {
            cycles.insert(url, member_cycles);
        }

        let ambiguous = self.ambiguous_by_note.pin();
        ambiguous.clear();
        for (url, endpoints) in built.ambiguous_by_note {
            ambiguous.insert(Self::normalize_url_key(&url), endpoints);
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

    /// Returns the hierarchical cycles this note is a member of (empty when it
    /// is not caught in one).
    pub fn cycles_for(&self, url: &str) -> Vec<RelationshipCycle> {
        self.cycles_by_note
            .pin()
            .get(&Self::normalize_url_key(url))
            .cloned()
            .unwrap_or_default()
    }

    /// Whether this note is a member of a hierarchical relationship cycle.
    ///
    /// Cheaper than [`Self::cycles_for`] (no clone) for callers that only need
    /// to know whether to warn.
    pub fn is_in_cycle(&self, url: &str) -> bool {
        self.cycles_by_note
            .pin()
            .get(&Self::normalize_url_key(url))
            .is_some_and(|cycles| !cycles.is_empty())
    }

    /// Returns the endpoints *this* note declared that resolved through a name
    /// shared by several notes (empty when none).
    pub fn ambiguous_endpoints_for(&self, url: &str) -> Vec<AmbiguousEndpoint> {
        self.ambiguous_by_note
            .pin()
            .get(&Self::normalize_url_key(url))
            .cloned()
            .unwrap_or_default()
    }

    /// Clears the index, including the detected data problems.
    pub fn clear(&self) {
        self.by_note.pin().clear();
        self.cycles_by_note.pin().clear();
        self.ambiguous_by_note.pin().clear();
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
    /// Normalised name -> every note URL answering to it (sorted), for names
    /// owned by **more than one** note. Unambiguous names are absent, so the
    /// map is empty for the overwhelmingly common case and reporting costs one
    /// miss per endpoint.
    ///
    /// Detection only: resolution still goes through `by_title` / `by_alias` /
    /// `by_stem` above, so first-wins behaviour is untouched.
    ambiguous: HashMap<String, Vec<String>>,
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
        // Every note answering to each name, deduped by URL so a note whose
        // title and filename stem normalise alike is not "ambiguous" with
        // itself. `notes` arrives sorted by URL, so each list is sorted too.
        let mut owners: HashMap<String, Vec<String>> = HashMap::new();
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

            // An index note's stem is a filename artifact ("index"), not a name
            // anyone means, and every repository with several `index.md` files
            // has one per folder — counting them would report "index" as
            // ambiguous in almost every repo. The title still participates, and
            // resolution is untouched (`by_stem` above indexes every note).
            let stem_as_name = (!note.is_index).then_some(&note.stem);
            let names = std::iter::once(&note.title)
                .chain(&note.aliases)
                .chain(stem_as_name);
            for name in names {
                let entry = owners.entry(normalize_name(name)).or_default();
                if !entry.contains(&note.url) {
                    entry.push(note.url.clone());
                }
            }
        }
        let ambiguous = owners
            .into_iter()
            .filter(|(_, urls)| urls.len() > 1)
            .collect();
        Self {
            by_title,
            by_alias,
            by_stem,
            urls,
            title_by_url,
            ambiguous,
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

        // Name-shaped endpoints (wikilink or bare name) go through the name
        // index; path-shaped ones resolve through the URL space instead.
        if let Some(name) = lookup_name(trimmed, markdown_extensions) {
            return self.resolve_by_name(name, raw);
        }

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
        ResolvedEndpoint::unresolved(trimmed.to_string(), raw)
    }

    /// The ambiguity record for an endpoint that resolved through a name owned
    /// by more than one note, or `None` when the name is unique, the endpoint
    /// did not resolve, or it was path-shaped (paths never consult the name
    /// index, so they are never ambiguous).
    ///
    /// Derives the looked-up name with the very same [`lookup_name`] that
    /// [`Self::resolve`] uses, so the two can never disagree about which name
    /// was consulted.
    fn ambiguity_for(
        &self,
        raw: &str,
        resolved: &ResolvedEndpoint,
        markdown_extensions: &[String],
    ) -> Option<AmbiguousEndpoint> {
        if !resolved.resolved || self.ambiguous.is_empty() {
            return None;
        }
        let name = lookup_name(raw.trim(), markdown_extensions)?;
        let owners = self.ambiguous.get(&normalize_name(name))?;
        let candidates: Vec<String> = owners
            .iter()
            .filter(|url| **url != resolved.url)
            .cloned()
            .collect();
        (!candidates.is_empty()).then(|| AmbiguousEndpoint {
            raw: raw.trim().to_string(),
            resolved_to: resolved.url.clone(),
            candidates,
        })
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

/// The name [`NameIndex::resolve`] looks up for an endpoint, or `None` when the
/// endpoint is path-shaped (relative/absolute file path), which resolves through
/// the URL space instead of the name index.
///
/// Handles `[[Name]]` and `[[Target|Alias]]` (the target before the pipe wins),
/// falling through to the bare string.
fn lookup_name<'a>(trimmed: &'a str, markdown_extensions: &[String]) -> Option<&'a str> {
    if let Some(inner) = strip_wikilink(trimmed) {
        return Some(inner.split('|').next().unwrap_or(inner).trim());
    }
    if looks_like_path(trimmed, markdown_extensions) {
        return None;
    }
    Some(trimmed)
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

/// Everything a relationship build produces: the per-note map plus the
/// data-quality findings mbr reports back to the author.
///
/// The findings are computed here rather than on the request path because a
/// rebuild already has the whole graph in hand and happens once per scan.
pub struct RelationshipBuild {
    /// url -> resolved relationships (declared + derived), deduped.
    pub by_note: BTreeMap<String, Vec<ResolvedRelationship>>,
    /// Hierarchical cycles, one entry per cycle.
    pub cycles: Vec<RelationshipCycle>,
    /// Declaring note url -> the endpoints it declared that resolved through a
    /// name shared by several notes.
    pub ambiguous_by_note: BTreeMap<String, Vec<AmbiguousEndpoint>>,
}

/// Builds the full relationship map (url -> resolved relationships) from notes.
///
/// Modelled on the build-mode inbound-link inversion: each declared edge is
/// attributed to both endpoints, with the reverse materialised via the
/// registry. Reciprocal declarations are deduped by a canonical edge key.
///
/// Convenience wrapper over [`build_relationships`] for callers that only need
/// the map.
pub fn build_relationship_map(
    notes: &[NoteRelInput],
    registry: &RelationTypeRegistry,
    markdown_extensions: &[String],
) -> BTreeMap<String, Vec<ResolvedRelationship>> {
    build_relationships(notes, registry, markdown_extensions).by_note
}

/// Builds the relationship map *and* the data-quality findings, warning about
/// each finding once.
///
/// See [`build_relationship_map`] for how the map itself is derived. On top of
/// that this reports the two ways relationship data goes quietly wrong:
///
/// - **Hierarchical cycles** ([`detect_relationship_cycles`]) — impossible data
///   that hangs the genealogy chart.
/// - **Ambiguous endpoint names** — an endpoint that named a title/alias/stem
///   shared by several notes and was resolved arbitrarily.
///
/// Runs once per index rebuild (server scan / file change / static build), never
/// on the request path.
pub fn build_relationships(
    notes: &[NoteRelInput],
    registry: &RelationTypeRegistry,
    markdown_extensions: &[String],
) -> RelationshipBuild {
    // Sort notes by URL for deterministic ambiguous-name resolution and stable
    // canonical-edge orientation.
    let mut sorted: Vec<&NoteRelInput> = notes.iter().collect();
    sorted.sort_by(|a, b| a.url.cmp(&b.url));

    let name_index = NameIndex::build(&sorted);

    let mut edges: BTreeMap<(String, String, String), AggregatedEdge> = BTreeMap::new();
    // Ambiguity findings, collected twice over: per declaring note (for
    // `errors.json`) and per name (to warn once regardless of how many notes
    // referenced it). Both are `BTreeMap`s so the output order is stable.
    let mut ambiguous_by_note: BTreeMap<String, Vec<AmbiguousEndpoint>> = BTreeMap::new();
    let mut ambiguous_by_name: BTreeMap<String, AmbiguousEndpoint> = BTreeMap::new();

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

            // Only *authored* endpoints can be ambiguous: an omitted `to`/`from`
            // means "this note", which is never a name lookup.
            let mut resolve_authored = |authored: &str| {
                let endpoint =
                    name_index.resolve(authored, &note.url, note.is_index, markdown_extensions);
                if let Some(found) =
                    name_index.ambiguity_for(authored, &endpoint, markdown_extensions)
                {
                    record_ambiguity(
                        &mut ambiguous_by_note,
                        &mut ambiguous_by_name,
                        &note.url,
                        found,
                    );
                }
                endpoint
            };

            let subject = match &rel.from {
                Some(s) => resolve_authored(s),
                None => self_ep(),
            };
            let object = match &rel.to {
                Some(s) => resolve_authored(s),
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

    let cycles = detect_relationship_cycles(&result, registry);
    warn_relationship_cycles(&cycles);
    warn_ambiguous_names(
        "relationship endpoint",
        &ambiguous_by_name
            .iter()
            .map(|(name, found)| AmbiguousNameReport {
                name,
                winner: &found.resolved_to,
                others: found.candidates.iter().map(String::as_str).collect(),
            })
            .collect::<Vec<_>>(),
    );

    RelationshipBuild {
        by_note: result,
        cycles,
        ambiguous_by_note,
    }
}

/// Records an ambiguous endpoint against the note that declared it *and* against
/// the per-name table used for the capped warning summary.
///
/// Per-note entries are deduped so a note declaring the same ambiguous endpoint
/// twice (e.g. as both a `parent` and a `spouse`) reports it once.
fn record_ambiguity(
    per_note: &mut BTreeMap<String, Vec<AmbiguousEndpoint>>,
    per_name: &mut BTreeMap<String, AmbiguousEndpoint>,
    declarer: &str,
    found: AmbiguousEndpoint,
) {
    per_name
        .entry(normalize_name(&found.raw))
        .or_insert_with(|| found.clone());
    let entries = per_note.entry(declarer.to_string()).or_default();
    if !entries.contains(&found) {
        entries.push(found);
    }
}

/// Logs one warning per detected cycle, naming every member so the author knows
/// exactly which notes to open.
fn warn_relationship_cycles(cycles: &[RelationshipCycle]) {
    for cycle in cycles {
        // Repeat the first member at the end so the chain visibly closes.
        let chain: Vec<&str> = cycle
            .members
            .iter()
            .chain(cycle.members.first())
            .map(String::as_str)
            .collect();
        tracing::warn!(
            "relationship cycle over `{}`: {} — each note is the previous note's \
             `{}`, which cannot be true in a hierarchy (nobody is their own \
             ancestor). The genealogy chart cannot be drawn for these notes until \
             one of those `{}` relationships is removed or corrected.",
            cycle.rel_type,
            chain.join(" -> "),
            cycle.rel_type,
            cycle.rel_type,
        );
    }
}

/// Detects cycles over the **hierarchical** (inverse-pair) edges of a built
/// relationship map.
///
/// Symmetric types (`spouse`, `sibling`) and plain directed/unknown types are
/// excluded: a spouse "cycle" is just a married couple, and an untyped edge
/// carries no hierarchy promise. Only inverse pairs describe an ordering that a
/// cycle can contradict.
///
/// Runs iterative Tarjan SCC per canonical relation type, then reduces each
/// component to one concrete simple cycle. O(V+E) in the hierarchical edge set,
/// with no recursion — a deep ancestor chain in a large repository must not be
/// able to blow the stack of a server worker thread.
///
/// Output is deterministic: edges are collected into `BTreeMap`/`BTreeSet`, so
/// node numbering, traversal order, and the reported cycle are stable across
/// runs.
pub fn detect_relationship_cycles(
    map: &BTreeMap<String, Vec<ResolvedRelationship>>,
    registry: &RelationTypeRegistry,
) -> Vec<RelationshipCycle> {
    let mut cycles = Vec::new();
    for (rel_type, edges) in hierarchical_edges(map, registry) {
        let graph = NodeGraph::from_edges(&edges);
        for component in strongly_connected_components(&graph.adjacency) {
            // A component is cyclic when it has several members, or when a lone
            // member points at itself. Self-edges should be impossible (the
            // build skips self-referential relationships) but cost nothing to
            // handle and would otherwise be reported as "no cycle".
            let self_looped = component.len() == 1
                && component
                    .first()
                    .is_some_and(|&node| graph.adjacency[node].contains(&node));
            if component.len() < 2 && !self_looped {
                continue;
            }
            let members = simple_cycle_in(&component, &graph.adjacency)
                .into_iter()
                .map(|node| graph.nodes[node].to_string())
                .collect();
            cycles.push(RelationshipCycle {
                members,
                rel_type: rel_type.clone(),
            });
        }
    }
    cycles
}

/// Extracts the hierarchical edge set of a built relationship map, grouped by
/// canonical relation type.
///
/// An edge `(subject, object)` reads "object is subject's `<canonical type>`" —
/// the same orientation [`canonical_key`] picks. Because both endpoints of one
/// logical edge carry a materialised entry, and their viewpoint predicates are
/// exact inverses, the two entries always yield the *identical* directed edge and
/// collapse in the `BTreeSet`.
fn hierarchical_edges(
    map: &BTreeMap<String, Vec<ResolvedRelationship>>,
    registry: &RelationTypeRegistry,
) -> BTreeMap<String, BTreeSet<(String, String)>> {
    let mut by_type: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();
    for (url, rels) in map {
        for rel in rels {
            if !rel.resolved || rel.neighbor.is_empty() {
                continue;
            }
            // The viewpoint predicate already names the neighbour's role, so it
            // is the right label to classify: for a declared `parent` edge the
            // object side sees `child` and vice versa.
            let predicate = rel.predicate.to_lowercase();
            if registry.is_symmetric(&predicate) {
                continue;
            }
            let Some(inverse) = registry.inverse_of(&predicate) else {
                continue;
            };
            let inverse = inverse.to_lowercase();
            // `inverse != predicate` is guaranteed here: a self-inverse type is
            // coerced to symmetric by `coerce_self_inverse` and so was already
            // skipped above. That matters, because such a pair's two halves are
            // indistinguishable and would otherwise look like a 2-cycle.
            let (canon, edge) = if predicate <= inverse {
                (predicate, (url.clone(), rel.neighbor.clone()))
            } else {
                (inverse, (rel.neighbor.clone(), url.clone()))
            };
            by_type
                .entry(registry.canonical_name(&canon))
                .or_default()
                .insert(edge);
        }
    }
    by_type
}

/// A directed graph over note URLs, interned to `usize` node ids so traversal
/// works on flat `Vec`s instead of hashing strings.
struct NodeGraph<'a> {
    /// Node id -> note URL.
    nodes: Vec<&'a str>,
    /// Node id -> outgoing neighbour ids.
    adjacency: Vec<Vec<usize>>,
}

impl<'a> NodeGraph<'a> {
    /// Interns the endpoints of `edges` in iteration order. `edges` is a
    /// `BTreeSet`, so ids (and therefore every traversal below) are assigned
    /// deterministically.
    fn from_edges(edges: &'a BTreeSet<(String, String)>) -> Self {
        let mut ids: HashMap<&str, usize> = HashMap::new();
        let mut nodes: Vec<&str> = Vec::new();
        for (from, to) in edges {
            for endpoint in [from.as_str(), to.as_str()] {
                if let std::collections::hash_map::Entry::Vacant(slot) = ids.entry(endpoint) {
                    slot.insert(nodes.len());
                    nodes.push(endpoint);
                }
            }
        }
        let mut adjacency = vec![Vec::new(); nodes.len()];
        for (from, to) in edges {
            adjacency[ids[from.as_str()]].push(ids[to.as_str()]);
        }
        Self { nodes, adjacency }
    }
}

/// Iterative Tarjan strongly-connected components.
///
/// Iterative on purpose: this runs on the server's index-rebuild path, and a
/// recursive version's stack depth is bounded by the longest ancestor chain in
/// the repository, which is user data.
fn strongly_connected_components(adjacency: &[Vec<usize>]) -> Vec<Vec<usize>> {
    const UNVISITED: usize = usize::MAX;
    let node_count = adjacency.len();
    let mut index = vec![UNVISITED; node_count];
    let mut lowlink = vec![UNVISITED; node_count];
    let mut on_stack = vec![false; node_count];
    let mut component_stack: Vec<usize> = Vec::new();
    // (node, index of the next outgoing edge to visit) — the explicit call stack.
    let mut call_stack: Vec<(usize, usize)> = Vec::new();
    let mut next_index = 0usize;
    let mut components: Vec<Vec<usize>> = Vec::new();

    for root in 0..node_count {
        if index[root] != UNVISITED {
            continue;
        }
        index[root] = next_index;
        lowlink[root] = next_index;
        next_index += 1;
        component_stack.push(root);
        on_stack[root] = true;
        call_stack.push((root, 0));

        while let Some(&(node, edge_cursor)) = call_stack.last() {
            if edge_cursor < adjacency[node].len() {
                if let Some(frame) = call_stack.last_mut() {
                    frame.1 += 1;
                }
                let next = adjacency[node][edge_cursor];
                if index[next] == UNVISITED {
                    index[next] = next_index;
                    lowlink[next] = next_index;
                    next_index += 1;
                    component_stack.push(next);
                    on_stack[next] = true;
                    call_stack.push((next, 0));
                } else if on_stack[next] {
                    lowlink[node] = lowlink[node].min(index[next]);
                }
                continue;
            }

            // Every edge explored: `node` is a component root when nothing
            // below it reached an earlier node.
            call_stack.pop();
            if lowlink[node] == index[node] {
                let mut component = Vec::new();
                while let Some(member) = component_stack.pop() {
                    on_stack[member] = false;
                    component.push(member);
                    if member == node {
                        break;
                    }
                }
                components.push(component);
            }
            if let Some(&(parent, _)) = call_stack.last() {
                lowlink[parent] = lowlink[parent].min(lowlink[node]);
            }
        }
    }

    components
}

/// Reduces a strongly-connected component to one concrete simple cycle.
///
/// Reporting the whole component would hand a genealogy author a list of every
/// note tangled in the knot; `A -> B -> C -> A` is what they can actually go and
/// fix. Iterative depth-first walk restricted to the component, started from its
/// smallest node id so the result is deterministic.
///
/// A strongly-connected component of two or more nodes always contains a cycle,
/// so the walk is guaranteed to find a back edge; the component itself is
/// returned as a defensive fallback rather than reporting nothing.
fn simple_cycle_in(component: &[usize], adjacency: &[Vec<usize>]) -> Vec<usize> {
    let members: HashSet<usize> = component.iter().copied().collect();
    let Some(&start) = component.iter().min() else {
        return Vec::new();
    };

    let mut path: Vec<usize> = vec![start];
    // node -> its position in `path` (the "grey" set of classic cycle detection).
    let mut position_on_path: HashMap<usize, usize> = HashMap::from([(start, 0)]);
    // Next outgoing edge to try, one entry per `path` entry.
    let mut cursors: Vec<usize> = vec![0];
    // Nodes already expanded; skipping them keeps the walk O(V+E).
    let mut visited: HashSet<usize> = HashSet::from([start]);

    while let Some(&node) = path.last() {
        let Some(&cursor) = cursors.last() else {
            break;
        };
        if cursor >= adjacency[node].len() {
            path.pop();
            cursors.pop();
            position_on_path.remove(&node);
            continue;
        }
        if let Some(slot) = cursors.last_mut() {
            *slot += 1;
        }
        let next = adjacency[node][cursor];
        if !members.contains(&next) {
            continue;
        }
        if let Some(&position) = position_on_path.get(&next) {
            return path[position..].to_vec();
        }
        if !visited.insert(next) {
            continue;
        }
        position_on_path.insert(next, path.len());
        path.push(next);
        cursors.push(0);
    }

    component.to_vec()
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

    /// A relation type with only `name` + `inverse` set, as a user would write
    /// `{ name = "employer", inverse = "employee" }` in `.mbr/config.toml`.
    fn inverse_type(name: &str, inverse: &str) -> RelationType {
        RelationType {
            name: name.to_string(),
            symmetric: false,
            inverse: Some(inverse.to_string()),
            label: None,
            label_plural: None,
        }
    }

    /// A symmetric relation type with only `name` + `symmetric` set.
    fn symmetric_type(name: &str) -> RelationType {
        RelationType {
            name: name.to_string(),
            symmetric: true,
            inverse: None,
            label: None,
            label_plural: None,
        }
    }

    /// The half-declared registry from the review: only the `employer` side of
    /// the pair is configured, plus an unrelated symmetric type.
    fn half_declared_types() -> Vec<RelationType> {
        vec![
            inverse_type("employer", "employee"),
            symmetric_type("spouse"),
        ]
    }

    fn rel_to(rel_type: &str, to: &str) -> RawRelationship {
        RawRelationship {
            rel_type: rel_type.to_string(),
            to: Some(to.to_string()),
            from: None,
            label: None,
            attributes: BTreeMap::new(),
        }
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
    fn registry_auto_registers_missing_inverse_half() {
        // Regression: only the `employer` half of the pair is configured, so
        // `employee` used to be unknown and `predicate_object("employee")` fell
        // back to "employee" — labelling the employer's page "Employees".
        let reg = RelationTypeRegistry::from_types(&half_declared_types());
        assert_eq!(reg.inverse_of("employee").as_deref(), Some("employer"));
        assert_eq!(reg.predicate_object("employee"), "employer");
        assert_eq!(reg.predicate_object("employer"), "employee");
        assert_eq!(reg.predicate_subject("employee"), "employee");
        assert!(!reg.is_symmetric("employee"));
        // The symmetric type is untouched by the derivation.
        assert!(reg.is_symmetric("spouse"));
        assert_eq!(reg.predicate_object("spouse"), "spouse");
        // Derived types are exposed to the frontend with auto-derived labels.
        let json = reg.to_json();
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        let employee = arr.iter().find(|t| t["name"] == "employee").unwrap();
        assert_eq!(employee["inverse"], "employer");
        assert_eq!(employee["label"], "Employee");
        assert_eq!(employee["label_plural"], "Employees");
        assert_eq!(employee["symmetric"], false);
    }

    #[test]
    fn registry_keeps_configured_types_over_derived_ones() {
        // `employer` claims `employee` as its inverse, but `employee` claims
        // `staff`. The explicit declarations win (and a warning is logged);
        // `staff` is auto-registered because nothing declares it.
        let reg = RelationTypeRegistry::from_types(&[
            inverse_type("employer", "employee"),
            inverse_type("employee", "staff"),
        ]);
        assert_eq!(reg.inverse_of("employee").as_deref(), Some("staff"));
        assert_eq!(reg.predicate_object("employee"), "staff");
        assert_eq!(reg.predicate_object("employer"), "employee");
        assert_eq!(reg.inverse_of("staff").as_deref(), Some("employee"));
    }

    #[test]
    fn registry_symmetric_wins_over_inverse_and_blank_inverse_ignored() {
        // `symmetric` and `inverse` are mutually exclusive; symmetric wins, so
        // no reciprocal is derived from the stray `inverse`.
        let reg = RelationTypeRegistry::from_types(&[
            RelationType {
                name: "partner".to_string(),
                symmetric: true,
                inverse: Some("partnered-with".to_string()),
                label: None,
                label_plural: None,
            },
            inverse_type("boss", "   "),
        ]);
        assert!(reg.get("partnered-with").is_none());
        assert_eq!(reg.predicate_object("partner"), "partner");
        // A blank `inverse` registers nothing and leaves the type directed.
        assert_eq!(reg.predicate_object("boss"), "boss");
        assert_eq!(reg.to_json().as_array().unwrap().len(), 2);
    }

    #[test]
    fn registry_coerces_self_inverse_type_to_symmetric() {
        // `inverse` names the *other* half of a pair, so naming yourself is
        // self-contradictory. Treat it as the symmetric type it describes.
        let reg = RelationTypeRegistry::from_types(&[inverse_type("sponsor", "sponsor")]);
        assert!(reg.is_symmetric("sponsor"));
        assert_eq!(reg.inverse_of("sponsor"), None);
        // Both viewpoints now agree, which is what stops the false 2-cycle.
        assert_eq!(reg.predicate_subject("sponsor"), "sponsor");
        assert_eq!(reg.predicate_object("sponsor"), "sponsor");
        // No phantom reciprocal was auto-registered from the stray `inverse`.
        assert_eq!(reg.to_json().as_array().unwrap().len(), 1);
    }

    #[test]
    fn registry_self_inverse_coercion_reaches_site_json() {
        // This is the propagation path that fixes the frontend without any TS
        // change: `classifyRelationship` tests `symmetric` before `inverse`, so
        // the coerced flag makes it take the symmetric branch.
        let reg = RelationTypeRegistry::from_types(&[inverse_type("sponsor", "sponsor")]);
        let json = reg.to_json();
        let sponsor = json
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "sponsor")
            .expect("type should be present");
        assert_eq!(sponsor["symmetric"], true);
        assert_eq!(sponsor["inverse"], serde_json::Value::Null);
    }

    #[test]
    fn registry_self_inverse_coercion_is_case_insensitive() {
        // `inverse = "Sponsor"` for `name = "sponsor"` is the same declaration;
        // relation-type lookup is case-insensitive everywhere else too.
        let reg = RelationTypeRegistry::from_types(&[inverse_type("sponsor", "Sponsor")]);
        assert!(reg.is_symmetric("sponsor"));
        assert_eq!(reg.inverse_of("sponsor"), None);

        // Also with surrounding whitespace and the casing reversed.
        let reg = RelationTypeRegistry::from_types(&[inverse_type("Peer", "  peer  ")]);
        assert!(reg.is_symmetric("peer"));
        assert_eq!(reg.inverse_of("peer"), None);
    }

    #[test]
    fn registry_self_inverse_coercion_warns_naming_the_type() {
        let (_reg, logs) = crate::test_support::capture_tracing(|| {
            RelationTypeRegistry::from_types(&[inverse_type("sponsor", "sponsor")])
        });
        assert!(
            logs.contains("`sponsor` names itself as its own `inverse`"),
            "the warning must name the offending type, got: {logs}"
        );
        assert!(
            logs.contains("symmetric = true"),
            "the warning must say what to do instead, got: {logs}"
        );
    }

    #[test]
    fn registry_normal_inverse_pair_is_untouched_by_coercion() {
        // The genealogy defaults and half-declared pairs must be completely
        // unaffected: only a *self*-naming inverse is coerced.
        let reg = RelationTypeRegistry::from_types(&genealogy_types());
        assert!(!reg.is_symmetric("parent"));
        assert!(!reg.is_symmetric("child"));
        assert_eq!(reg.inverse_of("parent").as_deref(), Some("child"));
        assert_eq!(reg.inverse_of("child").as_deref(), Some("parent"));

        let reg = RelationTypeRegistry::from_types(&half_declared_types());
        assert!(!reg.is_symmetric("employer"));
        assert_eq!(reg.inverse_of("employer").as_deref(), Some("employee"));
        assert_eq!(reg.inverse_of("employee").as_deref(), Some("employer"));

        // And a self-inverse type alongside normal ones only affects itself.
        let mut types = genealogy_types();
        types.push(inverse_type("sponsor", "sponsor"));
        let reg = RelationTypeRegistry::from_types(&types);
        assert!(reg.is_symmetric("sponsor"));
        assert_eq!(reg.inverse_of("parent").as_deref(), Some("child"));
        assert!(!reg.is_symmetric("parent"));
    }

    #[test]
    fn registry_defaults_are_unchanged_by_derivation() {
        // The genealogy defaults declare both halves, so nothing is derived.
        let reg = RelationTypeRegistry::from_types(&genealogy_types());
        assert_eq!(reg.to_json().as_array().unwrap().len(), 4);
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
    fn half_declared_inverse_labels_the_object_side() {
        // Regression: with only `{ name = "employer", inverse = "employee" }`
        // configured, B declaring "A is my employee" left A's page showing B
        // under *Employees* instead of *Employers*.
        let notes = vec![
            note("/a/", "A", "a", vec![]),
            note("/b/", "B", "b", vec![rel_to("employee", "[[A]]")]),
        ];
        let reg = RelationTypeRegistry::from_types(&half_declared_types());
        let map = build_relationship_map(&notes, &reg, &["md".to_string()]);

        let b = map.get("/b/").unwrap();
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].predicate, "employee");
        assert_eq!(b[0].neighbor, "/a/");
        assert!(!b[0].derived);

        let a = map.get("/a/").unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].predicate, "employer");
        assert_eq!(a[0].neighbor, "/b/");
        assert!(a[0].derived);
    }

    #[test]
    fn half_declared_reciprocal_declarations_collapse_to_one_edge() {
        // Regression: A says "B is my employer", B says "A is my employee".
        // With only the `employee` half configured the two declarations used to
        // land on different canonical keys, so each note carried two entries
        // for the same logical edge (one of them mislabelled).
        let notes = vec![
            note("/a/", "A", "a", vec![rel_to("employer", "[[B]]")]),
            note("/b/", "B", "b", vec![rel_to("employee", "[[A]]")]),
        ];
        let reg = RelationTypeRegistry::from_types(&[inverse_type("employee", "employer")]);
        let map = build_relationship_map(&notes, &reg, &["md".to_string()]);

        let a = map.get("/a/").unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].predicate, "employer");
        assert_eq!(a[0].neighbor, "/b/");
        // Both notes declared this edge, so neither side is derived.
        assert!(!a[0].derived);

        let b = map.get("/b/").unwrap();
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].predicate, "employee");
        assert_eq!(b[0].neighbor, "/a/");
        assert!(!b[0].derived);
    }

    #[test]
    fn half_declared_registry_still_handles_symmetric_types() {
        // The derivation must not disturb symmetric types sharing the registry.
        let notes = vec![
            note("/john/", "John", "john", vec![rel_to("spouse", "[[Mary]]")]),
            note("/mary/", "Mary", "mary", vec![]),
        ];
        let reg = RelationTypeRegistry::from_types(&half_declared_types());
        let map = build_relationship_map(&notes, &reg, &["md".to_string()]);
        let john = map.get("/john/").unwrap();
        assert_eq!(john.len(), 1);
        assert_eq!(john[0].predicate, "spouse");
        let mary = map.get("/mary/").unwrap();
        assert_eq!(mary.len(), 1);
        assert_eq!(mary[0].predicate, "spouse");
        assert!(mary[0].derived);
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

    // ----- cycle detection -----

    /// Builds with the default genealogy types and returns the findings.
    fn build_genealogy(notes: &[NoteRelInput]) -> RelationshipBuild {
        let reg = RelationTypeRegistry::from_types(&genealogy_types());
        build_relationships(notes, &reg, &["md".to_string()])
    }

    #[test]
    fn cycle_detection_finds_two_note_cycle() {
        // A says B is its parent and B says A is its parent. Canonically both
        // orient onto `child`, giving B -> A and A -> B: each is the other's
        // ancestor, which cannot be.
        let notes = vec![
            note("/a/", "A", "a", vec![rel_to("parent", "[[B]]")]),
            note("/b/", "B", "b", vec![rel_to("parent", "[[A]]")]),
        ];
        let built = build_genealogy(&notes);
        assert_eq!(built.cycles.len(), 1, "{:?}", built.cycles);
        let cycle = &built.cycles[0];
        assert_eq!(cycle.rel_type, "child");
        assert_eq!(cycle.members.len(), 2, "{:?}", cycle.members);
        assert!(cycle.members.contains(&"/a/".to_string()), "{:?}", cycle);
        assert!(cycle.members.contains(&"/b/".to_string()), "{:?}", cycle);
    }

    #[test]
    fn cycle_detection_finds_three_note_cycle() {
        let notes = vec![
            note("/a/", "A", "a", vec![rel_to("parent", "[[B]]")]),
            note("/b/", "B", "b", vec![rel_to("parent", "[[C]]")]),
            note("/c/", "C", "c", vec![rel_to("parent", "[[A]]")]),
        ];
        let built = build_genealogy(&notes);
        assert_eq!(built.cycles.len(), 1, "{:?}", built.cycles);
        let mut members = built.cycles[0].members.clone();
        members.sort();
        assert_eq!(members, vec!["/a/", "/b/", "/c/"]);
    }

    #[test]
    fn cycle_detection_ignores_acyclic_tree() {
        // A textbook family: one parent, two children, one grandchild.
        let notes = vec![
            note(
                "/root/",
                "Root",
                "root",
                vec![rel_to("child", "[[Kid1]]"), rel_to("child", "[[Kid2]]")],
            ),
            note("/kid1/", "Kid1", "kid1", vec![rel_to("child", "[[Grand]]")]),
            note("/kid2/", "Kid2", "kid2", vec![]),
            note("/grand/", "Grand", "grand", vec![]),
        ];
        let built = build_genealogy(&notes);
        assert!(built.cycles.is_empty(), "{:?}", built.cycles);
    }

    #[test]
    fn cycle_detection_ignores_symmetric_only_cycle() {
        // Spouses and siblings reference each other by design; a "cycle" over
        // symmetric edges is just a family, not bad data.
        let notes = vec![
            note(
                "/a/",
                "A",
                "a",
                vec![rel_to("spouse", "[[B]]"), rel_to("sibling", "[[C]]")],
            ),
            note("/b/", "B", "b", vec![rel_to("spouse", "[[A]]")]),
            note("/c/", "C", "c", vec![rel_to("sibling", "[[A]]")]),
        ];
        let built = build_genealogy(&notes);
        assert!(built.cycles.is_empty(), "{:?}", built.cycles);
    }

    #[test]
    fn cycle_detection_ignores_self_inverse_type() {
        // `{ name = "rival", inverse = "rival" }` is self-contradictory. It is
        // coerced to symmetric at registry construction, so the symmetric check
        // excludes it and no special case is needed here — but left uncoerced its
        // two indistinguishable halves would look like a 2-cycle for every pair.
        let reg = RelationTypeRegistry::from_types(&[inverse_type("rival", "rival")]);
        assert!(reg.is_symmetric("rival"), "coercion is what prevents this");

        let notes = vec![
            note("/a/", "A", "a", vec![rel_to("rival", "[[B]]")]),
            note("/b/", "B", "b", vec![]),
        ];
        let built = build_relationships(&notes, &reg, &["md".to_string()]);
        // The edge itself is still tracked, once on each note...
        assert_eq!(built.by_note.get("/a/").unwrap().len(), 1);
        assert_eq!(built.by_note.get("/b/").unwrap().len(), 1);
        // ...but it is not reported as a cycle.
        assert!(built.cycles.is_empty(), "{:?}", built.cycles);
    }

    #[test]
    fn cycle_detection_ignores_mutually_declared_self_inverse_type() {
        // The shape the coordinator reproduced: both notes declare the type, so
        // *both* viewpoints exist. Uncoerced this produced two dedup keys and a
        // false "each is the other's ancestor" for every such pair.
        let reg = RelationTypeRegistry::from_types(&[inverse_type("sponsor", "sponsor")]);
        let notes = vec![
            note(
                "/people/one/",
                "One",
                "one",
                vec![rel_to("sponsor", "[[Two]]")],
            ),
            note(
                "/people/two/",
                "Two",
                "two",
                vec![rel_to("sponsor", "[[One]]")],
            ),
        ];
        let built = build_relationships(&notes, &reg, &["md".to_string()]);
        assert!(built.cycles.is_empty(), "{:?}", built.cycles);
        // Symmetric handling also collapses the reciprocal declarations into a
        // single edge per note, rather than keeping two half-edges.
        assert_eq!(built.by_note.get("/people/one/").unwrap().len(), 1);
        assert_eq!(built.by_note.get("/people/two/").unwrap().len(), 1);
    }

    #[test]
    fn cycle_detection_ignores_unknown_directed_type() {
        // An unregistered type promises no hierarchy, so mutual `mentor` edges
        // are not an error.
        let notes = vec![
            note("/a/", "A", "a", vec![rel_to("mentor", "[[B]]")]),
            note("/b/", "B", "b", vec![rel_to("mentor", "[[A]]")]),
        ];
        let built = build_genealogy(&notes);
        assert!(built.cycles.is_empty(), "{:?}", built.cycles);
    }

    #[test]
    fn cycle_detection_reports_one_cycle_per_component() {
        // Two loops sharing note A form a single strongly-connected component,
        // so the author gets one actionable report rather than one per loop.
        let notes = vec![
            note(
                "/a/",
                "A",
                "a",
                vec![rel_to("child", "[[B]]"), rel_to("child", "[[C]]")],
            ),
            note("/b/", "B", "b", vec![rel_to("child", "[[A]]")]),
            note("/c/", "C", "c", vec![rel_to("child", "[[A]]")]),
        ];
        let built = build_genealogy(&notes);
        assert_eq!(built.cycles.len(), 1, "{:?}", built.cycles);
        // The report is a concrete simple cycle, not the whole component.
        assert_eq!(built.cycles[0].members.len(), 2, "{:?}", built.cycles);
    }

    #[test]
    fn cycle_detection_is_deterministic() {
        // Same input, byte-identical findings — the warning text and the
        // errors.json payload must not reshuffle between runs.
        let notes = vec![
            note("/a/", "A", "a", vec![rel_to("parent", "[[B]]")]),
            note("/b/", "B", "b", vec![rel_to("parent", "[[C]]")]),
            note("/c/", "C", "c", vec![rel_to("parent", "[[A]]")]),
            note("/x/", "X", "x", vec![rel_to("parent", "[[Y]]")]),
            note("/y/", "Y", "y", vec![rel_to("parent", "[[X]]")]),
        ];
        let first = build_genealogy(&notes);
        for _ in 0..5 {
            let again = build_genealogy(&notes);
            assert_eq!(first.cycles, again.cycles);
        }
        assert_eq!(first.cycles.len(), 2, "{:?}", first.cycles);
    }

    #[test]
    fn cycle_detection_ignores_unresolved_endpoints() {
        // An endpoint that resolved to nothing has no node to close a loop with.
        let notes = vec![note("/a/", "A", "a", vec![rel_to("parent", "[[Nobody]]")])];
        let built = build_genealogy(&notes);
        assert!(built.cycles.is_empty(), "{:?}", built.cycles);
    }

    #[test]
    fn cycle_detection_separates_distinct_relation_types() {
        // A `child` loop and an `employer` loop are independent findings.
        let mut types = genealogy_types();
        types.push(inverse_type("employer", "employee"));
        let reg = RelationTypeRegistry::from_types(&types);
        let notes = vec![
            note(
                "/a/",
                "A",
                "a",
                vec![rel_to("child", "[[B]]"), rel_to("employer", "[[B]]")],
            ),
            note(
                "/b/",
                "B",
                "b",
                vec![rel_to("child", "[[A]]"), rel_to("employer", "[[A]]")],
            ),
        ];
        let built = build_relationships(&notes, &reg, &["md".to_string()]);
        let mut kinds: Vec<&str> = built
            .cycles
            .iter()
            .map(|cycle| cycle.rel_type.as_str())
            .collect();
        kinds.sort_unstable();
        assert_eq!(kinds, vec!["child", "employee"], "{:?}", built.cycles);
    }

    // ----- ambiguous endpoint names -----

    #[test]
    fn ambiguous_endpoint_is_recorded_against_the_declaring_note() {
        // The classic genealogy trap: a John Doe Sr and a John Doe Jr.
        let notes = vec![
            note("/people/john-jr/", "John Doe", "john-jr", vec![]),
            note("/people/john-sr/", "John Doe", "john-sr", vec![]),
            note("/z/", "Zeb", "zeb", vec![rel_to("parent", "[[John Doe]]")]),
        ];
        let built = build_genealogy(&notes);

        let found = built
            .ambiguous_by_note
            .get("/z/")
            .expect("declaring note should carry the finding");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].raw, "[[John Doe]]");
        assert_eq!(found[0].resolved_to, "/people/john-jr/");
        assert_eq!(found[0].candidates, vec!["/people/john-sr/".to_string()]);

        // Resolution behaviour is unchanged: the smallest URL still wins.
        assert_eq!(
            built.by_note.get("/z/").unwrap()[0].neighbor,
            "/people/john-jr/"
        );
        // The namesakes themselves declared nothing, so nothing is attached to
        // them.
        assert!(!built.ambiguous_by_note.contains_key("/people/john-sr/"));
        assert!(!built.ambiguous_by_note.contains_key("/people/john-jr/"));
    }

    #[test]
    fn unambiguous_endpoint_records_nothing() {
        let notes = vec![
            note("/people/mary/", "Mary Doe", "mary", vec![]),
            note("/z/", "Zeb", "zeb", vec![rel_to("spouse", "[[Mary Doe]]")]),
        ];
        let built = build_genealogy(&notes);
        assert!(
            built.ambiguous_by_note.is_empty(),
            "{:?}",
            built.ambiguous_by_note
        );
    }

    #[test]
    fn note_answering_to_a_name_twice_is_not_ambiguous_with_itself() {
        // "alpha" is both the title and the filename stem of the *same* note:
        // one owner, not two.
        let notes = vec![
            note("/a/", "alpha", "alpha", vec![]),
            note("/z/", "Zeb", "zeb", vec![rel_to("parent", "[[alpha]]")]),
        ];
        let built = build_genealogy(&notes);
        assert!(
            built.ambiguous_by_note.is_empty(),
            "{:?}",
            built.ambiguous_by_note
        );
    }

    #[test]
    fn path_endpoint_is_never_reported_as_ambiguous() {
        // Naming the file explicitly is precisely the fix the warning suggests,
        // so a path endpoint must never itself be flagged.
        let notes = vec![
            note("/other/mary/", "Mary", "mary-other", vec![]),
            note(
                "/people/john/",
                "John",
                "john",
                vec![rel_to("parent", "mary.md")],
            ),
            note("/people/mary/", "Mary", "mary", vec![]),
        ];
        let built = build_genealogy(&notes);
        assert_eq!(
            built.by_note.get("/people/john/").unwrap()[0].neighbor,
            "/people/mary/"
        );
        assert!(
            built.ambiguous_by_note.is_empty(),
            "{:?}",
            built.ambiguous_by_note
        );
    }

    #[test]
    fn ambiguous_endpoint_via_alias_is_reported() {
        // "Mary Doe" is one note's title and another's maiden-name alias.
        let mut alias_holder = note("/people/b-mary/", "Mary Smith", "b-mary", vec![]);
        alias_holder.aliases = vec!["Mary Doe".into()];
        let notes = vec![
            note("/people/a-mary/", "Mary Doe", "a-mary", vec![]),
            alias_holder,
            note("/z/", "Zeb", "zeb", vec![rel_to("spouse", "[[Mary Doe]]")]),
        ];
        let built = build_genealogy(&notes);
        let found = built.ambiguous_by_note.get("/z/").expect("finding");
        assert_eq!(found[0].resolved_to, "/people/a-mary/");
        assert_eq!(found[0].candidates, vec!["/people/b-mary/".to_string()]);
    }

    #[test]
    fn index_note_stems_are_not_reported_as_ambiguous() {
        // Same rationale as the wikilink index: "index" is a filename artifact
        // shared by every folder's index note, so it must not be treated as an
        // ambiguous name. Resolution is unchanged.
        let mut docs = note("/docs/", "Docs", "index", vec![]);
        docs.is_index = true;
        let mut guide = note("/guide/", "Guide", "index", vec![]);
        guide.is_index = true;
        let notes = vec![
            docs,
            guide,
            note("/z/", "Zeb", "zeb", vec![rel_to("parent", "[[index]]")]),
        ];
        let built = build_genealogy(&notes);
        // Still resolves first-wins...
        assert_eq!(built.by_note.get("/z/").unwrap()[0].neighbor, "/docs/");
        // ...but is not reported.
        assert!(
            built.ambiguous_by_note.is_empty(),
            "{:?}",
            built.ambiguous_by_note
        );
    }

    #[test]
    fn ambiguous_name_warnings_are_capped_with_a_summary() {
        // A genealogy repo can have dozens of namesakes; the log must stay
        // readable while errors.json stays complete.
        let ambiguous_count = AMBIGUOUS_WARN_CAP + 5;
        let mut notes = Vec::new();
        for i in 0..ambiguous_count {
            // Two notes share each title, and a third references it.
            notes.push(note(
                &format!("/a{i:02}/"),
                &format!("Dup {i:02}"),
                &format!("a{i:02}"),
                vec![],
            ));
            notes.push(note(
                &format!("/b{i:02}/"),
                &format!("Dup {i:02}"),
                &format!("b{i:02}"),
                vec![],
            ));
            notes.push(note(
                &format!("/ref{i:02}/"),
                &format!("Ref {i:02}"),
                &format!("ref{i:02}"),
                vec![rel_to("parent", &format!("[[Dup {i:02}]]"))],
            ));
        }

        let (built, logs) = crate::test_support::capture_tracing(|| build_genealogy(&notes));

        // Every affected page still gets its own complete finding.
        assert_eq!(built.ambiguous_by_note.len(), ambiguous_count);

        let individual = logs
            .matches("ambiguous relationship endpoint name `")
            .count();
        assert_eq!(individual, AMBIGUOUS_WARN_CAP, "{logs}");
        assert!(
            logs.contains("and 5 more ambiguous relationship endpoint names"),
            "{logs}"
        );
    }

    #[test]
    fn cycle_warning_names_every_member_and_the_relation_type() {
        let notes = vec![
            note("/a/", "A", "a", vec![rel_to("parent", "[[B]]")]),
            note("/b/", "B", "b", vec![rel_to("parent", "[[A]]")]),
        ];
        let (_built, logs) = crate::test_support::capture_tracing(|| build_genealogy(&notes));
        assert!(logs.contains("relationship cycle over `child`"), "{logs}");
        assert!(logs.contains("/a/"), "{logs}");
        assert!(logs.contains("/b/"), "{logs}");
    }

    // ----- index-level queries -----

    #[test]
    fn index_exposes_cycles_per_member_note() {
        let index = RelationshipIndex::from_relation_types(&genealogy_types());
        index.rebuild(
            &[
                note("/a/", "A", "a", vec![rel_to("parent", "[[B]]")]),
                note("/b/", "B", "b", vec![rel_to("parent", "[[A]]")]),
                note("/c/", "C", "c", vec![rel_to("parent", "[[A]]")]),
            ],
            &["md".to_string()],
        );

        assert!(index.is_in_cycle("/a/"));
        assert!(index.is_in_cycle("/b/"));
        // C points into the cycle but is not caught in it.
        assert!(!index.is_in_cycle("/c/"));
        assert_eq!(index.cycles_for("/a/").len(), 1);
        assert_eq!(index.cycles_for("/a/")[0].rel_type, "child");
        assert!(index.cycles_for("/c/").is_empty());
        // Lookups normalise the leading slash like `get` does.
        assert_eq!(index.cycles_for("a/").len(), 1);

        // A rebuild with corrected data clears the finding.
        index.rebuild(
            &[
                note("/a/", "A", "a", vec![rel_to("parent", "[[B]]")]),
                note("/b/", "B", "b", vec![]),
            ],
            &["md".to_string()],
        );
        assert!(!index.is_in_cycle("/a/"));
        assert!(!index.is_in_cycle("/b/"));
    }

    #[test]
    fn index_exposes_ambiguous_endpoints_per_declaring_note() {
        let index = RelationshipIndex::from_relation_types(&genealogy_types());
        index.rebuild(
            &[
                note("/people/john-jr/", "John Doe", "john-jr", vec![]),
                note("/people/john-sr/", "John Doe", "john-sr", vec![]),
                note("/z/", "Zeb", "zeb", vec![rel_to("parent", "[[John Doe]]")]),
            ],
            &["md".to_string()],
        );

        let found = index.ambiguous_endpoints_for("/z/");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].resolved_to, "/people/john-jr/");
        assert_eq!(found[0].candidates, vec!["/people/john-sr/".to_string()]);
        assert!(index.ambiguous_endpoints_for("/people/john-sr/").is_empty());

        index.clear();
        assert!(index.ambiguous_endpoints_for("/z/").is_empty());
        assert!(!index.is_in_cycle("/z/"));
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
