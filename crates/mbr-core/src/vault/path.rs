//! [`VaultPath`]: a normalized, root-relative path that cannot name anything
//! outside its vault.

use std::fmt;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

/// Why a string or native path is not a valid [`VaultPath`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VaultPathError {
    /// A `..` would climb above the vault root.
    #[error("path escapes the vault root: {0:?}")]
    Escapes(String),
    /// The path is absolute: a leading `/` or `\`, a UNC share, a drive
    /// (`C:`), or a native path with a root or prefix component.
    #[error("path is absolute: {0:?}")]
    Absolute(String),
    /// The path contains a NUL byte, which no filesystem accepts and which
    /// truncates the path at the C boundary.
    #[error("path contains a NUL byte: {0:?}")]
    Nul(String),
    /// A segment is not a plain file name on this host — on Windows, a segment
    /// such as `Q:x` parses as a drive-relative prefix.
    #[error("path segment {segment:?} is not a plain file name: {path:?}")]
    InvalidSegment { path: String, segment: String },
    /// A native path component is not valid UTF-8. Vault paths are strings so
    /// that they mean the same thing on every platform and over every storage
    /// backend (Android SAF and sync libraries have no `OsStr`).
    #[error("path is not valid UTF-8: {0:?}")]
    NotUtf8(PathBuf),
    /// An absolute path handed to [`VaultPath::from_absolute`] is not under the
    /// given root.
    #[error("{path:?} is not under the vault root {root:?}")]
    NotUnderRoot { path: PathBuf, root: PathBuf },
}

/// A path inside a vault: root-relative, `/`-separated and normalized.
///
/// # Invariants
///
/// Every value of this type, however constructed, satisfies all of:
///
/// - it is either empty (the vault root itself) or a sequence of segments
///   joined by a single `/`, with no leading or trailing `/`;
/// - no segment is empty, `.` or `..`;
/// - no segment contains `/`, `\` or NUL;
/// - no segment is a bare drive designator (`C:`), on any platform;
/// - every segment is a single plain file-name component on the host
///   ([`std::path::Component::Normal`]), so [`VaultPath::to_native`] can only
///   ever produce a path *under* the root it is given.
///
/// Together these mean a `VaultPath` cannot name anything outside its vault
/// **lexically**. Symlinks are a separate question, answered where untrusted
/// input enters: [`super::Vault::canonicalize`] resolves them and reports
/// [`super::VaultError::OutsideRoot`] for a link that leaves the vault. Reads
/// do not re-check, for the reason the desktop scanner never has: it reads
/// every file in the repository, and a `realpath` per read would cost more than
/// the read.
///
/// # Separators
///
/// Parsing treats both `/` and `\` as separators on every platform. A request
/// such as `a\..\..\etc` therefore normalizes (and is refused) identically on
/// Unix and Windows; the cost is that a Unix file whose name contains a
/// backslash is not addressable through a vault, which no portable repository
/// can contain anyway.
///
/// # Percent-encoding
///
/// None is decoded here. URL paths reach the resolver already decoded by axum,
/// and authored hrefs go through
/// [`crate::path_resolver::normalize_link_target`] first; decoding again would
/// turn a literal `%2e%2e` file name into a traversal.
#[derive(Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VaultPath(String);

impl VaultPath {
    /// The vault root itself (the empty path).
    pub fn root() -> Self {
        Self(String::new())
    }

    /// Parses a root-relative path, normalizing it lexically.
    ///
    /// Accepts `/`- or `\`-separated input. `.` segments and repeated or
    /// trailing separators are dropped; `..` pops the previous segment and is
    /// refused when there is nothing left to pop. A leading separator, a UNC
    /// share or a drive designator is refused as [`VaultPathError::Absolute`]:
    /// the callers of this function (request paths, repo-relative paths) are
    /// relative by construction, so an absolute value is never a spelling of
    /// something inside the vault.
    pub fn new(path: &str) -> Result<Self, VaultPathError> {
        if path.contains('\0') {
            return Err(VaultPathError::Nul(path.to_string()));
        }
        if path.starts_with(['/', '\\']) {
            return Err(VaultPathError::Absolute(path.to_string()));
        }

        // Built in place: this runs per request and per resolved path, so it
        // allocates once and never per segment.
        let mut normalized = String::with_capacity(path.len());
        for segment in path.split(['/', '\\']) {
            match segment {
                "" | "." => {}
                ".." => {
                    if !pop_segment(&mut normalized) {
                        return Err(VaultPathError::Escapes(path.to_string()));
                    }
                }
                _ => {
                    check_segment(segment).map_err(|problem| problem.error(path, segment))?;
                    push_segment(&mut normalized, segment);
                }
            }
        }
        Ok(Self(normalized))
    }

    /// Converts a repo-relative **native** path (such as
    /// [`crate::repo::MarkdownInfo::raw_path`]) into a vault path.
    ///
    /// `.` is dropped and `..` pops, as in [`Self::new`]; a root or prefix
    /// component, or a non-UTF-8 component, is refused.
    pub fn from_relative_native(path: &Path) -> Result<Self, VaultPathError> {
        let mut normalized = String::with_capacity(path.as_os_str().len());
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    if !pop_segment(&mut normalized) {
                        return Err(VaultPathError::Escapes(path.display().to_string()));
                    }
                }
                Component::Normal(name) => {
                    let name = name
                        .to_str()
                        .ok_or_else(|| VaultPathError::NotUtf8(path.to_path_buf()))?;
                    // A native component can still hold a `\` on Unix, which
                    // would become a separator on the way back out.
                    if name.contains('\\') {
                        return Err(VaultPathError::InvalidSegment {
                            path: path.display().to_string(),
                            segment: name.to_string(),
                        });
                    }
                    check_segment(name)
                        .map_err(|problem| problem.error(&path.display().to_string(), name))?;
                    push_segment(&mut normalized, name);
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(VaultPathError::Absolute(path.display().to_string()));
                }
            }
        }
        Ok(Self(normalized))
    }

    /// Converts an absolute native path under `root` into a vault path.
    ///
    /// Containment is component-wise ([`Path::strip_prefix`]), never textual,
    /// and purely lexical: `root` and `path` must be spelled in the same base —
    /// both canonical, in the vault's case, which is what
    /// [`super::LocalVault`] guarantees for the paths it produces.
    pub fn from_absolute(root: &Path, path: &Path) -> Result<Self, VaultPathError> {
        let relative = path
            .strip_prefix(root)
            .map_err(|_| VaultPathError::NotUnderRoot {
                path: path.to_path_buf(),
                root: root.to_path_buf(),
            })?;
        Self::from_relative_native(relative)
    }

    /// The path as a `/`-separated string; empty for the root.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this is the vault root.
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// The segments, outermost first. Empty for the root.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('/').filter(|segment| !segment.is_empty())
    }

    /// The last segment, or `None` for the root.
    pub fn file_name(&self) -> Option<&str> {
        if self.is_root() {
            None
        } else {
            self.0.rsplit('/').next()
        }
    }

    /// The final extension of [`Self::file_name`], as [`Path::extension`]
    /// defines it (`archive.tar.gz` → `gz`; `.hidden` → none).
    pub fn extension(&self) -> Option<&str> {
        Path::new(self.file_name()?)
            .extension()
            .and_then(|ext| ext.to_str())
    }

    /// The containing directory, or `None` for the root.
    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        Some(match self.0.rfind('/') {
            Some(idx) => Self(self.0[..idx].to_string()),
            None => Self::root(),
        })
    }

    /// This path and each ancestor, innermost first, ending at the root.
    pub fn ancestors(&self) -> impl Iterator<Item = Self> {
        std::iter::successors(Some(self.clone()), Self::parent)
    }

    /// Appends a relative path (parsed with [`Self::new`], so it may hold
    /// several segments or `..`). A `..` may climb back out of `self` but never
    /// past the vault root.
    pub fn join(&self, relative: &str) -> Result<Self, VaultPathError> {
        if self.is_root() {
            return Self::new(relative);
        }
        if relative.starts_with(['/', '\\']) {
            return Err(VaultPathError::Absolute(relative.to_string()));
        }
        Self::new(&format!("{}/{relative}", self.0))
    }

    /// Appends another vault path. Infallible: both sides already satisfy the
    /// invariants, and concatenating valid segments cannot break them.
    pub fn join_path(&self, other: &Self) -> Self {
        match (self.is_root(), other.is_root()) {
            (_, true) => self.clone(),
            (true, false) => other.clone(),
            (false, false) => Self(format!("{}/{}", self.0, other.0)),
        }
    }

    /// Appends exactly one segment — a directory entry's own name. Unlike
    /// [`Self::join`], separators are refused rather than split on, so a Unix
    /// file called `a\b` cannot become the two segments `a/b`.
    pub fn child(&self, name: &str) -> Result<Self, VaultPathError> {
        let full = if self.is_root() {
            name.to_string()
        } else {
            format!("{}/{name}", self.0)
        };
        if name.contains('\0') {
            return Err(VaultPathError::Nul(full));
        }
        if name.contains(['/', '\\']) {
            return Err(VaultPathError::InvalidSegment {
                path: full,
                segment: name.to_string(),
            });
        }
        check_segment(name).map_err(|problem| problem.error(&full, name))?;
        Ok(Self(full))
    }

    /// Whether `prefix` is this path or one of its ancestors (segment-wise:
    /// `a/bc` does not start with `a/b`).
    pub fn starts_with(&self, prefix: &Self) -> bool {
        prefix.is_root()
            || self.0 == prefix.0
            || (self.0.starts_with(&prefix.0)
                && self.0.as_bytes().get(prefix.0.len()) == Some(&b'/'))
    }

    /// The native path for this vault path under `root`.
    ///
    /// Built by pushing one validated segment at a time, so the result is
    /// always `root` itself or a descendant of it: no segment can be a root,
    /// a prefix or a `..` (see the type's invariants).
    pub fn to_native(&self, root: &Path) -> PathBuf {
        let mut native = PathBuf::with_capacity(root.as_os_str().len() + self.0.len() + 1);
        native.push(root);
        for segment in self.segments() {
            native.push(segment);
        }
        native
    }
}

/// Appends one segment to a normalized path under construction.
fn push_segment(normalized: &mut String, segment: &str) {
    if !normalized.is_empty() {
        normalized.push('/');
    }
    normalized.push_str(segment);
}

/// Drops the last segment of a normalized path under construction; `false`
/// when there was none (a `..` at the root).
fn pop_segment(normalized: &mut String) -> bool {
    if normalized.is_empty() {
        return false;
    }
    let cut = normalized.rfind('/').unwrap_or(0);
    normalized.truncate(cut);
    true
}

/// What is wrong with a segment, before the error naming it is built — so the
/// hot path never formats a message it does not need.
enum SegmentProblem {
    Invalid,
    BareDrive,
}

impl SegmentProblem {
    fn error(self, path: &str, segment: &str) -> VaultPathError {
        match self {
            Self::Invalid => VaultPathError::InvalidSegment {
                path: path.to_string(),
                segment: segment.to_string(),
            },
            Self::BareDrive => VaultPathError::Absolute(path.to_string()),
        }
    }
}

/// Validates one non-empty, non-`.`/`..` segment.
fn check_segment(segment: &str) -> Result<(), SegmentProblem> {
    if segment.is_empty() || segment == "." || segment == ".." {
        return Err(SegmentProblem::Invalid);
    }
    // A bare drive designator is refused on every platform, so `C:/Windows`
    // means the same thing (nothing) everywhere. Only the *bare* form: a Unix
    // note called `Q: what next.md` is an ordinary file name there.
    if is_bare_drive(segment) {
        return Err(SegmentProblem::BareDrive);
    }
    // What the host itself would parse the segment as. On Windows `Q:x` is a
    // drive-relative prefix and `PathBuf::push` would *replace* the root with
    // it; on Unix it is a plain name. Either way only a single `Normal`
    // component can be pushed under a root without leaving it.
    let mut components = Path::new(segment).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(name)), None) if name == segment => Ok(()),
        _ => Err(SegmentProblem::Invalid),
    }
}

/// `C:` — an ASCII letter followed by a colon, and nothing else.
fn is_bare_drive(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

impl fmt::Debug for VaultPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VaultPath({:?})", self.0)
    }
}

impl fmt::Display for VaultPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for VaultPath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl serde::Serialize for VaultPath {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl TryFrom<&str> for VaultPath {
    type Error = VaultPathError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn vp(s: &str) -> VaultPath {
        VaultPath::new(s).unwrap_or_else(|e| panic!("{s:?}: {e}"))
    }

    #[test]
    fn normalizes_separators_dots_and_trailing_slashes() {
        assert_eq!(vp("a/b/c.md").as_str(), "a/b/c.md");
        assert_eq!(vp("./a//b/./c.md/").as_str(), "a/b/c.md");
        assert_eq!(vp("a\\b\\c.md").as_str(), "a/b/c.md");
        assert_eq!(vp("a/x/../b").as_str(), "a/b");
        assert_eq!(vp("").as_str(), "");
        assert_eq!(vp(".").as_str(), "");
        assert_eq!(vp("a/..").as_str(), "");
        assert!(vp("").is_root());
    }

    #[test]
    fn refuses_traversal_above_the_root() {
        for input in [
            "..",
            "../x",
            "a/../../x",
            "a/b/../../../etc/passwd",
            "..\\x",
            "a\\..\\..\\x",
            "docs/./../../x",
        ] {
            assert!(
                matches!(VaultPath::new(input), Err(VaultPathError::Escapes(_))),
                "{input:?} should escape"
            );
        }
    }

    #[test]
    fn refuses_absolute_unc_and_drive_forms_on_every_platform() {
        for input in [
            "/etc/passwd",
            "\\etc\\passwd",
            "//server/share/x",
            "\\\\server\\share\\x",
            "\\\\?\\C:\\x",
            "C:",
            "C:/Windows/system32",
            "C:\\Windows",
            "docs/C:/x",
            "z:",
        ] {
            assert!(
                matches!(VaultPath::new(input), Err(VaultPathError::Absolute(_))),
                "{input:?} should be absolute, got {:?}",
                VaultPath::new(input)
            );
        }
    }

    #[test]
    fn refuses_nul() {
        assert!(matches!(
            VaultPath::new("a\0b"),
            Err(VaultPathError::Nul(_))
        ));
    }

    /// Percent-encoded traversal is *not* decoded: `%2e%2e` is a literal file
    /// name, contained like any other.
    #[test]
    fn encoded_traversal_is_a_literal_name_that_stays_inside() {
        let p = vp("%2e%2e/%2e%2e/etc/passwd");
        assert_eq!(p.as_str(), "%2e%2e/%2e%2e/etc/passwd");
        let root = Path::new("/vault");
        assert!(p.to_native(root).starts_with(root));
        let p = vp("..%2f..%2fetc");
        assert_eq!(p.segments().count(), 1);
    }

    /// The decoded form of an encoded traversal, which is what
    /// `normalize_link_target` hands the resolver, is refused.
    #[test]
    fn decoded_traversal_is_refused() {
        let decoded = crate::path_resolver::normalize_link_target("/%2e%2e/%2e%2e/etc/passwd");
        assert!(VaultPath::new(&decoded).is_err(), "{decoded}");
        let decoded = crate::path_resolver::normalize_link_target("/a/%2e%2e%2f%2e%2e%2fb");
        assert!(VaultPath::new(&decoded).is_err(), "{decoded}");
    }

    /// A colon is an ordinary file-name character wherever the host says so.
    #[test]
    fn colon_names_follow_the_host() {
        let parsed = VaultPath::new("Q: what next.md");
        if cfg!(windows) {
            assert!(parsed.is_err());
        } else {
            assert_eq!(parsed.unwrap().as_str(), "Q: what next.md");
        }
        // Not drive-shaped anywhere.
        assert_eq!(vp("Re: notes.md").as_str(), "Re: notes.md");
    }

    #[test]
    fn accessors() {
        let p = vp("docs/guide/archive.tar.gz");
        assert_eq!(p.file_name(), Some("archive.tar.gz"));
        assert_eq!(p.extension(), Some("gz"));
        assert_eq!(p.parent(), Some(vp("docs/guide")));
        assert_eq!(vp("top.md").parent(), Some(VaultPath::root()));
        assert_eq!(VaultPath::root().parent(), None);
        assert_eq!(VaultPath::root().file_name(), None);
        assert_eq!(vp(".hidden").extension(), None);
        let ancestors: Vec<String> = p.ancestors().map(|a| a.to_string()).collect();
        assert_eq!(
            ancestors,
            vec!["docs/guide/archive.tar.gz", "docs/guide", "docs", ""]
        );
    }

    #[test]
    fn child_appends_exactly_one_segment() {
        assert_eq!(vp("a").child("b.md").unwrap(), vp("a/b.md"));
        assert_eq!(VaultPath::root().child("b.md").unwrap(), vp("b.md"));
        for bad in ["", ".", "..", "x/y", "x\\y", "C:", "nul\0"] {
            assert!(vp("a").child(bad).is_err(), "{bad:?}");
        }
        assert_eq!(vp("a").join_path(&vp("b/c")), vp("a/b/c"));
        assert_eq!(VaultPath::root().join_path(&vp("b")), vp("b"));
        assert_eq!(vp("a").join_path(&VaultPath::root()), vp("a"));
    }

    #[test]
    fn starts_with_is_segment_wise() {
        assert!(vp("a/b/c").starts_with(&vp("a/b")));
        assert!(vp("a/b").starts_with(&vp("a/b")));
        assert!(vp("a/b").starts_with(&VaultPath::root()));
        assert!(!vp("a/bc").starts_with(&vp("a/b")));
        assert!(!vp("a").starts_with(&vp("a/b")));
    }

    #[test]
    fn join_normalizes_but_never_escapes() {
        assert_eq!(vp("a/b").join("c.md").unwrap(), vp("a/b/c.md"));
        assert_eq!(vp("a/b").join("../c.md").unwrap(), vp("a/c.md"));
        assert_eq!(VaultPath::root().join("x").unwrap(), vp("x"));
        assert!(vp("a").join("../../x").is_err());
        assert!(vp("a").join("/etc").is_err());
    }

    #[test]
    fn native_round_trip() {
        let root = std::env::temp_dir().join("vault-root");
        let p = vp("docs/sub dir/note.md");
        let native = p.to_native(&root);
        assert!(native.starts_with(&root));
        assert_eq!(VaultPath::from_absolute(&root, &native).unwrap(), p);
        let relative = native.strip_prefix(&root).unwrap();
        assert_eq!(VaultPath::from_relative_native(relative).unwrap(), p);
        assert_eq!(VaultPath::root().to_native(&root), root);
    }

    #[test]
    fn from_absolute_refuses_paths_outside_the_root() {
        let root = std::env::temp_dir().join("vault-root");
        let sibling = std::env::temp_dir().join("vault-root-other").join("x.md");
        assert!(matches!(
            VaultPath::from_absolute(&root, &sibling),
            Err(VaultPathError::NotUnderRoot { .. })
        ));
        let climbing = root.join("..").join("x.md");
        assert!(VaultPath::from_absolute(&root, &climbing).is_err());
    }

    #[test]
    fn from_relative_native_refuses_rooted_paths() {
        let absolute = std::env::temp_dir().join("x.md");
        assert!(matches!(
            VaultPath::from_relative_native(&absolute),
            Err(VaultPathError::Absolute(_))
        ));
        assert!(VaultPath::from_relative_native(Path::new("../x.md")).is_err());
        assert_eq!(
            VaultPath::from_relative_native(Path::new("./a/../b.md")).unwrap(),
            vp("b.md")
        );
    }

    #[test]
    fn serializes_as_its_string() {
        assert_eq!(serde_json::to_string(&vp("a/b.md")).unwrap(), "\"a/b.md\"");
    }

    /// Arbitrary path-ish input over an alphabet weighted towards the
    /// characters that matter: separators, dots, colons and drive letters.
    fn pathish() -> impl Strategy<Value = String> {
        proptest::collection::vec(
            prop_oneof![
                Just("/".to_string()),
                Just("\\".to_string()),
                Just(".".to_string()),
                Just("..".to_string()),
                Just(":".to_string()),
                Just("C:".to_string()),
                Just("%2e".to_string()),
                Just(" ".to_string()),
                "[a-zA-Z0-9_-]{1,6}",
                "\\PC{1,3}",
            ],
            0..12,
        )
        .prop_map(|parts| parts.concat())
    }

    proptest! {
        /// Normalization is idempotent: a parsed path re-parses to itself.
        #[test]
        fn prop_normalization_is_idempotent(input in pathish()) {
            if let Ok(p) = VaultPath::new(&input) {
                prop_assert_eq!(VaultPath::new(p.as_str()).unwrap(), p);
            }
        }

        /// Whatever is accepted stays under the root once made native, and has
        /// no component that could climb, re-root or re-prefix it.
        #[test]
        fn prop_never_escapes_the_root(input in pathish()) {
            let root = std::env::temp_dir().join("vault-prop-root");
            if let Ok(p) = VaultPath::new(&input) {
                let native = p.to_native(&root);
                prop_assert!(native.starts_with(&root), "{:?} -> {:?}", input, native);
                let relative = native.strip_prefix(&root).unwrap();
                prop_assert!(relative.components().all(|c| matches!(c, Component::Normal(_))));
                prop_assert_eq!(relative.components().count(), p.segments().count());
                // The string form never carries anything a later parse would
                // read as traversal or as absolute.
                prop_assert!(!p.as_str().starts_with('/'));
                prop_assert!(!p.as_str().contains('\\'));
                prop_assert!(p.segments().all(|s| s != ".." && s != "." && !s.is_empty()));
            }
        }

        /// Native round trip: under any root, `to_native` then `from_absolute`
        /// is the identity.
        #[test]
        fn prop_round_trips_through_native(input in pathish()) {
            let root = std::env::temp_dir().join("vault-prop-root");
            if let Ok(p) = VaultPath::new(&input) {
                let native = p.to_native(&root);
                prop_assert_eq!(VaultPath::from_absolute(&root, &native).unwrap(), p);
            }
        }

        /// Any input carrying a `..` deeper than everything before it is
        /// refused, however it is separated.
        #[test]
        fn prop_rejects_leading_traversal(
            tail in "[a-z]{1,5}(/[a-z]{1,5}){0,3}",
            sep in prop_oneof![Just("/"), Just("\\")],
            depth in 1usize..4,
        ) {
            let input = format!("{}{tail}", format!("..{sep}").repeat(depth));
            prop_assert!(VaultPath::new(&input).is_err());
        }

        /// A `..` that only pops segments it was given is fine and lands on
        /// the lexical answer.
        #[test]
        fn prop_balanced_traversal_cancels(
            head in "[a-z]{1,5}(/[a-z]{1,5}){0,3}",
            name in "[a-z]{1,5}",
        ) {
            let depth = head.split('/').count();
            let input = format!("{head}/{}{name}", "../".repeat(depth));
            let parsed = VaultPath::new(&input).unwrap();
            prop_assert_eq!(parsed.as_str(), name.as_str());
        }

        /// Leading separators are absolute, never silently rooted inside.
        #[test]
        fn prop_rejects_absolute(
            tail in "[a-z]{0,5}(/[a-z]{1,5}){0,3}",
            lead in prop_oneof![Just("/"), Just("\\"), Just("//"), Just("\\\\")],
        ) {
            let absolute = format!("{lead}{tail}");
            prop_assert!(matches!(
                VaultPath::new(&absolute),
                Err(VaultPathError::Absolute(_))
            ));
        }

        /// `starts_with` agrees with `ancestors`.
        #[test]
        fn prop_ancestors_are_prefixes(input in "[a-z]{1,4}(/[a-z]{1,4}){0,5}") {
            let p = VaultPath::new(&input).unwrap();
            for ancestor in p.ancestors() {
                prop_assert!(p.starts_with(&ancestor));
            }
        }
    }
}
