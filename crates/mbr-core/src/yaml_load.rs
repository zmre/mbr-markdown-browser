//! Bounded YAML loading for note frontmatter.
//!
//! yaml-rust2's `YamlLoader` expands every `*alias` by deep-cloning the
//! anchored node, with no limit. Frontmatter is attacker-sized text that mbr
//! parses for every note on every scan, so a sub-kilobyte "billion laughs"
//! block —
//!
//! ```yaml
//! a: &a [x, x, x, x, x, x, x, x, x]
//! b: &b [*a, *a, *a, *a, *a, *a, *a, *a, *a]
//! c: &c [*b, *b, *b, *b, *b, *b, *b, *b, *b]
//! # …
//! ```
//!
//! — expands to ~10⁹ nodes and gets the process killed for memory, at every
//! launch, since the note is re-indexed each time.
//!
//! [`load_yaml_documents`] builds the same tree `YamlLoader` builds, from the
//! same parser events, in one pass, while charging every node it materializes
//! (scalars, collections, each node an alias copies, each anchored node it
//! remembers) against [`MAX_EXPANDED_NODES`]. The charge happens *before* the
//! copy, so a refused document never allocates past the budget; after a
//! refusal the remaining events are only scanned. Work and memory are
//! therefore `O(input + MAX_EXPANDED_NODES)`. It also caps the number of
//! aliases ([`MAX_ALIASES`]) and the nesting depth ([`MAX_DEPTH`]: the
//! tree is dropped and walked recursively, and `- - - - …` nests a level
//! every two bytes).
//!
//! Every non-test parse of note YAML goes through here; a refusal is a
//! [`YamlLoadError`] the callers treat exactly like a syntax error (the note
//! still lists and renders without its frontmatter, and the message reaches
//! the log and `errors.json`).

use std::collections::HashMap;
use std::mem;

use yaml_rust2::Yaml;
use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser, Tag};
use yaml_rust2::scanner::{Marker, ScanError, TScalarStyle};
use yaml_rust2::yaml::Hash;

/// Most aliases (`*name`) a document may use. Frontmatter that reuses a
/// value uses a handful; a hundred is far past any real note.
pub const MAX_ALIASES: usize = 100;

/// Most nodes a document may materialize, counting every copy an alias makes
/// (and the copy kept of each anchored node). A long frontmatter block is a
/// few hundred nodes.
pub const MAX_EXPANDED_NODES: usize = 100_000;

/// Deepest collection nesting a document may have.
pub const MAX_DEPTH: usize = 128;

/// Why a YAML text was not loaded.
#[derive(Debug, thiserror::Error)]
pub enum YamlLoadError {
    /// The text is not valid YAML (yaml-rust2's own error, message unchanged).
    #[error("{0}")]
    Syntax(#[from] ScanError),
    /// More than [`MAX_ALIASES`] aliases.
    #[error("frontmatter refused: too many YAML aliases (more than {MAX_ALIASES})")]
    TooManyAliases,
    /// Aliases expand the document past [`MAX_EXPANDED_NODES`] nodes.
    #[error("frontmatter refused: YAML aliases expand to more than {MAX_EXPANDED_NODES} values")]
    ExpansionTooLarge,
    /// Collections nested deeper than [`MAX_DEPTH`].
    #[error("frontmatter refused: YAML nested deeper than {MAX_DEPTH} levels")]
    TooDeep,
}

impl YamlLoadError {
    /// True for a refusal by one of this module's caps (as opposed to a YAML
    /// syntax error).
    pub fn is_refusal(&self) -> bool {
        !matches!(self, Self::Syntax(_))
    }
}

/// Loads every YAML document in `text`, exactly as
/// `yaml_rust2::YamlLoader::load_from_str` would, but refusing documents past
/// this module's caps.
///
/// # Errors
/// [`YamlLoadError::Syntax`] for invalid YAML (including a duplicated mapping
/// key, as `YamlLoader` reports it); the other variants for a refusal. A
/// refusal wins over a later syntax error.
pub fn load_yaml_documents(text: &str) -> Result<Vec<Yaml>, YamlLoadError> {
    let mut loader = BoundedLoader::default();
    let parsed = Parser::new_from_str(text).load(&mut loader, true);
    match (loader.error, parsed) {
        (Some(error), _) => Err(error),
        (None, Err(error)) => Err(error.into()),
        (None, Ok(())) => Ok(loader.docs),
    }
}

/// The first YAML document in `text`, or `None` when the text holds none (a
/// block of only comments). See [`load_yaml_documents`].
///
/// # Errors
/// As [`load_yaml_documents`].
pub fn load_first_yaml_document(text: &str) -> Result<Option<Yaml>, YamlLoadError> {
    load_yaml_documents(text).map(|docs| docs.into_iter().next())
}

/// An open collection.
struct Frame {
    node: Yaml,
    /// Anchor id (0 = none, as the parser numbers them).
    anchor: usize,
    /// Nodes in the collection so far, itself included, aliases expanded.
    size: usize,
    /// A mapping's pending key; `BadValue` means "the next node is a key",
    /// the same sentinel `YamlLoader` uses.
    key: Yaml,
}

#[derive(Default)]
struct BoundedLoader {
    docs: Vec<Yaml>,
    stack: Vec<Frame>,
    /// The finished top-level node of the current document.
    root: Option<Yaml>,
    /// Anchored nodes with their expanded size.
    anchors: HashMap<usize, (Yaml, usize)>,
    aliases: usize,
    materialized: usize,
    error: Option<YamlLoadError>,
}

impl MarkedEventReceiver for BoundedLoader {
    fn on_event(&mut self, event: Event, mark: Marker) {
        if self.error.is_none()
            && let Err(error) = self.handle(event, mark)
        {
            // Drop what was built: a refused document is never returned.
            self.stack.clear();
            self.anchors.clear();
            self.root = None;
            self.error = Some(error);
        }
    }
}

impl BoundedLoader {
    fn handle(&mut self, event: Event, mark: Marker) -> Result<(), YamlLoadError> {
        match event {
            Event::Nothing | Event::StreamStart | Event::StreamEnd | Event::DocumentStart => {}
            Event::DocumentEnd => {
                let doc = self.root.take().unwrap_or(Yaml::BadValue);
                self.docs.push(doc);
            }
            Event::SequenceStart(anchor, _) => self.open(Yaml::Array(Vec::new()), anchor)?,
            Event::MappingStart(anchor, _) => self.open(Yaml::Hash(Hash::new()), anchor)?,
            Event::SequenceEnd | Event::MappingEnd => {
                if let Some(frame) = self.stack.pop() {
                    self.finish(frame.node, frame.anchor, frame.size, mark)?;
                }
            }
            Event::Scalar(value, style, anchor, tag) => {
                self.charge(1)?;
                self.finish(resolve_scalar(value, style, tag.as_ref()), anchor, 1, mark)?;
            }
            Event::Alias(id) => {
                self.aliases += 1;
                if self.aliases > MAX_ALIASES {
                    return Err(YamlLoadError::TooManyAliases);
                }
                let size = self.anchors.get(&id).map_or(1, |(_, size)| *size);
                // Charged before the copy is made.
                self.charge(size)?;
                let node = self
                    .anchors
                    .get(&id)
                    .map_or(Yaml::BadValue, |(node, _)| node.clone());
                self.finish(node, 0, size, mark)?;
            }
        }
        Ok(())
    }

    fn charge(&mut self, nodes: usize) -> Result<(), YamlLoadError> {
        self.materialized = self.materialized.saturating_add(nodes);
        if self.materialized > MAX_EXPANDED_NODES {
            Err(YamlLoadError::ExpansionTooLarge)
        } else {
            Ok(())
        }
    }

    fn open(&mut self, node: Yaml, anchor: usize) -> Result<(), YamlLoadError> {
        if self.stack.len() >= MAX_DEPTH {
            return Err(YamlLoadError::TooDeep);
        }
        self.charge(1)?;
        self.stack.push(Frame {
            node,
            anchor,
            size: 1,
            key: Yaml::BadValue,
        });
        Ok(())
    }

    /// Places a finished node in its parent (or as the document's root),
    /// remembering it first when it carries an anchor. Mirrors
    /// `YamlLoader::insert_new_node`, duplicate-key error included.
    fn finish(
        &mut self,
        node: Yaml,
        anchor: usize,
        size: usize,
        mark: Marker,
    ) -> Result<(), YamlLoadError> {
        if anchor > 0 {
            self.charge(size)?;
            self.anchors.insert(anchor, (node.clone(), size));
        }
        let Some(parent) = self.stack.last_mut() else {
            self.root = Some(node);
            return Ok(());
        };
        parent.size = parent.size.saturating_add(size);
        match &mut parent.node {
            Yaml::Array(items) => items.push(node),
            Yaml::Hash(hash) => {
                if parent.key.is_badvalue() {
                    parent.key = node;
                } else {
                    let key = mem::replace(&mut parent.key, Yaml::BadValue);
                    if hash.insert(key, node).is_some() {
                        let inserted = hash.back().map_or(&Yaml::BadValue, |(key, _)| key);
                        return Err(ScanError::new_string(
                            mark,
                            format!("{inserted:?}: duplicated key in mapping"),
                        )
                        .into());
                    }
                }
            }
            // Only collections are ever pushed.
            _ => {}
        }
        Ok(())
    }
}

/// A scalar's value, resolved as `YamlLoader` resolves it.
fn resolve_scalar(value: String, style: TScalarStyle, tag: Option<&Tag>) -> Yaml {
    if style != TScalarStyle::Plain {
        return Yaml::String(value);
    }
    let Some(tag) = tag else {
        return Yaml::from_str(&value);
    };
    if tag.handle != "tag:yaml.org,2002:" {
        return Yaml::String(value);
    }
    match tag.suffix.as_str() {
        "bool" => match value.as_str() {
            "true" | "True" | "TRUE" => Yaml::Boolean(true),
            "false" | "False" | "FALSE" => Yaml::Boolean(false),
            _ => Yaml::BadValue,
        },
        "int" => value.parse::<i64>().map_or(Yaml::BadValue, Yaml::Integer),
        "float" => {
            if is_f64(&value) {
                Yaml::Real(value)
            } else {
                Yaml::BadValue
            }
        }
        "null" => match value.as_str() {
            "~" | "null" => Yaml::Null,
            _ => Yaml::BadValue,
        },
        _ => Yaml::String(value),
    }
}

/// yaml-rust2's (private) `parse_f64`, as a predicate: the Core schema's
/// float spellings.
fn is_f64(v: &str) -> bool {
    match v {
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" | "-.inf" | "-.Inf" | "-.INF"
        | ".nan" | ".NaN" | ".NAN" => true,
        _ => v.as_bytes().iter().any(u8::is_ascii_digit) && v.parse::<f64>().is_ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::time::{Duration, Instant};
    use yaml_rust2::YamlLoader;

    /// `levels` levels of nine aliases to the level below: 9^levels leaves.
    fn billion_laughs(levels: usize) -> String {
        let mut text = String::from("l0: &l0 [x, x, x, x, x, x, x, x, x]\n");
        for level in 1..levels {
            let prev = format!("*l{}", level - 1);
            let items = [prev.as_str(); 9].join(", ");
            text.push_str(&format!("l{level}: &l{level} [{items}]\n"));
        }
        text
    }

    /// The bomb's expanded size, computed the way the loader charges it,
    /// without building anything: what `YamlLoader` would allocate.
    fn bomb_expanded_nodes(levels: u32) -> u128 {
        // level 0: 1 seq + 9 scalars; level n: 1 + 9 * size(n-1)
        (1..levels).fold(10u128, |size, _| 1 + 9 * size)
    }

    fn assert_refused_quickly(text: &str) {
        let started = Instant::now();
        let result = load_yaml_documents(text);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "took {:?}",
            started.elapsed()
        );
        let error = result.expect_err("bomb must be refused");
        assert!(error.is_refusal(), "{error}");
        assert!(
            error.to_string().starts_with("frontmatter refused"),
            "{error}"
        );
    }

    #[test]
    fn nine_level_billion_laughs_is_refused() {
        // What the unpatched loader would build: ~430 million nodes.
        assert!(bomb_expanded_nodes(9) > 400_000_000);
        assert_refused_quickly(&billion_laughs(9));
    }

    #[test]
    fn twenty_level_billion_laughs_is_refused() {
        // 20 levels use 171 aliases: refused by expansion before the count.
        assert_refused_quickly(&billion_laughs(20));
    }

    #[test]
    fn expansion_cap_is_hit_before_alias_cap() {
        // 9 levels use 72 aliases, fewer than MAX_ALIASES: the expansion cap
        // is what stops it.
        assert!(matches!(
            load_yaml_documents(&billion_laughs(9)),
            Err(YamlLoadError::ExpansionTooLarge)
        ));
    }

    #[test]
    fn many_cheap_aliases_are_refused_by_count() {
        let mut text = String::from("a: &a x\nb: [");
        text.push_str(&vec!["*a"; MAX_ALIASES + 1].join(", "));
        text.push_str("]\n");
        assert!(matches!(
            load_yaml_documents(&text),
            Err(YamlLoadError::TooManyAliases)
        ));
        // Exactly the cap is fine.
        let ok = format!("a: &a x\nb: [{}]\n", vec!["*a"; MAX_ALIASES].join(", "));
        assert!(load_yaml_documents(&ok).is_ok());
    }

    #[test]
    fn deep_nesting_is_refused() {
        let deep = "- ".repeat(MAX_DEPTH + 1) + "x\n";
        assert!(matches!(
            load_yaml_documents(&deep),
            Err(YamlLoadError::TooDeep)
        ));
        let ok = "- ".repeat(MAX_DEPTH) + "x\n";
        assert!(load_yaml_documents(&ok).is_ok());
    }

    #[test]
    fn legitimate_anchors_match_yaml_loader() {
        for text in [
            "defaults: &d {lang: en, draft: false}\npost: *d\n",
            "base: &base\n  a: 1\n  b: [x, y]\nother: *base\nthird: *base\n",
            "tags: &t [rust, yaml]\nalso: *t\nscalar: &s hello\nagain: *s\n",
            "billion_laughs_lite: &a [x, x]\nb: [*a, *a]\n",
        ] {
            assert_eq!(
                load_yaml_documents(text).unwrap(),
                YamlLoader::load_from_str(text).unwrap(),
                "{text}"
            );
        }
    }

    #[test]
    fn edge_cases_match_yaml_loader() {
        for text in [
            "",
            "# only a comment",
            "---\n...\n",
            "a: 1\n---\nb: 2\n",
            "i: !!int 12\nbad: !!int x\nf: !!float 1.5\nbf: !!float y\nfi: !!float 12\n",
            "fx: !!float 0x1F\nfn: !!float .nan\nfinf: !!float -.Inf\nfe: !!float inf\n",
            "b: !!bool True\nn: !!null ~\nbn: !!null x\ns: !!str 12\nc: !custom 3\n",
            "q: '12'\nd: \"true\"\nlit: |\n  text\n",
            "inf: .inf\nnan: .NaN\nneg: -.inf\nnum: 1e3\n",
            "? [a, b]\n: complex key\n",
        ] {
            assert_eq!(
                format!("{:?}", load_yaml_documents(text).map_err(|e| e.to_string())),
                format!(
                    "{:?}",
                    YamlLoader::load_from_str(text).map_err(|e| e.to_string())
                ),
                "{text:?}"
            );
        }
    }

    #[test]
    fn errors_match_yaml_loader() {
        for text in ["a: [1, 2", "a: 1\na: 2\n", "key: *undefined\n", "a: b: c\n"] {
            let ours = load_yaml_documents(text);
            let theirs = YamlLoader::load_from_str(text);
            assert_eq!(
                ours.as_ref().err().map(ToString::to_string),
                theirs.as_ref().err().map(ToString::to_string),
                "{text:?}"
            );
            assert_eq!(ours.ok(), theirs.ok(), "{text:?}");
        }
    }

    #[test]
    fn first_document_is_none_for_comments_only() {
        assert!(
            load_first_yaml_document("# tags: [draft]")
                .unwrap()
                .is_none()
        );
        assert_eq!(
            load_first_yaml_document("a: 1\n---\nb: 2\n").unwrap(),
            YamlLoader::load_from_str("a: 1")
                .unwrap()
                .into_iter()
                .next()
        );
    }

    fn scalar() -> impl Strategy<Value = String> {
        prop_oneof![
            "[a-z]{1,8}",
            "-?[0-9]{1,6}",
            "-?[0-9]{1,3}\\.[0-9]{1,3}",
            Just("true".to_string()),
            Just("null".to_string()),
            Just("~".to_string()),
            "\"[a-z :#]{0,8}\"",
            "'[a-z ]{0,8}'",
        ]
    }

    fn flow_value() -> impl Strategy<Value = String> {
        let leaf = scalar();
        leaf.prop_recursive(3, 24, 4, |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..4)
                    .prop_map(|items| format!("[{}]", items.join(", "))),
                prop::collection::vec(("[a-z]{1,6}", inner), 0..4).prop_map(|pairs| {
                    let body = pairs
                        .iter()
                        .map(|(k, v)| format!("{k}: {v}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{{{body}}}")
                }),
            ]
        })
    }

    proptest! {
        /// Alias-free documents load to exactly what `YamlLoader` loads,
        /// errors (e.g. duplicate keys) included.
        #[test]
        fn alias_free_documents_match_yaml_loader(
            entries in prop::collection::vec(("[a-z]{1,6}", flow_value()), 0..8)
        ) {
            let text: String = entries
                .iter()
                .map(|(k, v)| format!("{k}: {v}\n"))
                .collect();
            let ours = load_yaml_documents(&text);
            let theirs = YamlLoader::load_from_str(&text);
            prop_assert_eq!(
                ours.as_ref().err().map(ToString::to_string),
                theirs.as_ref().err().map(ToString::to_string)
            );
            prop_assert_eq!(ours.ok(), theirs.ok());
        }
    }
}
