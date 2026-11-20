//! Markup an embedding application adds to the `<head>` of every page a
//! server renders: a theme (CSS custom properties) and/or raw HTML.
//!
//! An app that serves mbr's router inside its own (`mbr_server::server::
//! ServerConfig::theme_vars` / `extra_head_html`) can push its own look into
//! mbr's chrome without writing a `.mbr/` file into the user's repository.
//! The markup reaches pages through the `host_head()` Tera function, which
//! `_head.html` calls last and which renders nothing when there is no host
//! head (static builds, QuickLook, `mbr -o`). It is a function rather than a
//! context variable because every frontmatter key becomes a context variable:
//! a note could otherwise shadow it with markup of its own.
//!
//! **Trust.** `extra_head_html` is the host's own markup and is emitted
//! verbatim. Theme variables are validated ([`HostHead::new`]) so that no
//! value can leave its declaration, its rule or the `<style>` element:
//! a value has no `<`, `>`, `{`, `}`, `;`, `!`, `\`, control characters or
//! comments, and its quotes and parentheses are balanced.
//!
//! **CSP.** mbr sends no `Content-Security-Policy`. A host that adds one
//! needs the theme's inline `<style>` allowed; [`HostHead::style_csp_source`]
//! is its `'sha256-…'` source, stable for the same variables, so a policy can
//! name exactly this element instead of `'unsafe-inline'`.

use std::collections::BTreeMap;
use std::sync::Arc;

use sha2::{Digest, Sha256};

/// The `id` of the theme's `<style>` element.
pub const THEME_STYLE_ID: &str = "mbr-host-theme";

/// The longest theme variable value accepted, in bytes.
pub const MAX_THEME_VALUE_BYTES: usize = 1024;

/// The longest theme variable name accepted, in bytes (`--` included).
pub const MAX_THEME_NAME_BYTES: usize = 128;

/// Why a theme variable was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HostHeadError {
    /// Not `--` followed by ASCII letters, digits, `-` and `_`.
    #[error(
        "theme variable name {name:?} is not a CSS custom property \
         (`--` then ASCII letters, digits, `-` or `_`, at most {MAX_THEME_NAME_BYTES} bytes)"
    )]
    InvalidName { name: String },
    /// A value that could leave its declaration, or is empty or too long.
    #[error("theme variable `{name}`: {reason}")]
    InvalidValue { name: String, reason: &'static str },
}

/// Validated markup for every page's `<head>`: the theme's `<style>` element
/// (when there are variables), then the host's HTML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostHead {
    /// Everything emitted, in order.
    html: Arc<str>,
    /// The `<style>` element's text, for its CSP hash.
    style: Option<String>,
}

impl HostHead {
    /// Builds the head markup, or `None` when there is nothing to add.
    ///
    /// Each `theme_vars` entry becomes `name: value !important` in one
    /// `:root` rule. `!important` is deliberate: Pico declares its colours on
    /// `:root:not([data-theme=dark])` and friends, which outrank a plain
    /// `:root`, and a host theme is meant to win over Pico, `theme.css` and a
    /// repository's `user.css`. For separate light and dark colours, use CSS
    /// `light-dark(light, dark)` in the value: it follows the scheme Pico
    /// sets (`color-scheme`), including the reader's OS preference.
    ///
    /// # Errors
    ///
    /// [`HostHeadError`] for the first name or value that is refused (in
    /// name order).
    pub fn new(
        extra_head_html: Option<&str>,
        theme_vars: &BTreeMap<String, String>,
    ) -> Result<Option<Self>, HostHeadError> {
        let style = theme_style(theme_vars)?;
        let extra = extra_head_html.filter(|html| !html.trim().is_empty());
        if style.is_none() && extra.is_none() {
            return Ok(None);
        }
        let style_element = style
            .as_deref()
            .map(|css| format!("<style id=\"{THEME_STYLE_ID}\">{css}</style>"));
        let html: String = [style_element.as_deref(), extra]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Some(Self {
            html: html.into(),
            style,
        }))
    }

    /// The markup, exactly as pages emit it.
    pub fn html(&self) -> &str {
        &self.html
    }

    /// The CSP source allowing the theme's `<style>` element
    /// (`'sha256-<base64>'`, for `style-src`), or `None` without variables.
    /// `extra_head_html` is the host's own markup and is not covered: a host
    /// that puts inline styles or scripts there hashes them itself.
    pub fn style_csp_source(&self) -> Option<String> {
        self.style.as_deref().map(|css| {
            let digest = Sha256::digest(css.as_bytes());
            format!("'sha256-{}'", base64_standard(&digest))
        })
    }
}

/// The text of the theme's `<style>` element, or `None` without variables.
fn theme_style(theme_vars: &BTreeMap<String, String>) -> Result<Option<String>, HostHeadError> {
    if theme_vars.is_empty() {
        return Ok(None);
    }
    let declarations = theme_vars
        .iter()
        .map(|(name, value)| {
            check_name(name)?;
            let value = value.trim();
            check_value(name, value)?;
            Ok(format!("{name}: {value} !important;"))
        })
        .collect::<Result<Vec<_>, HostHeadError>>()?;
    Ok(Some(format!(":root {{ {} }}", declarations.join(" "))))
}

fn check_name(name: &str) -> Result<(), HostHeadError> {
    let valid = name.len() <= MAX_THEME_NAME_BYTES
        && name.strip_prefix("--").is_some_and(|rest| {
            !rest.is_empty()
                && rest
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        });
    if valid {
        Ok(())
    } else {
        Err(HostHeadError::InvalidName {
            name: name.to_string(),
        })
    }
}

/// Refuses anything that could end the declaration, the rule or the
/// `<style>` element, or swallow what follows it: a value is a run of CSS
/// tokens with balanced quotes and parentheses and none of the characters
/// that delimit those structures.
fn check_value(name: &str, value: &str) -> Result<(), HostHeadError> {
    let refuse = |reason: &'static str| {
        Err(HostHeadError::InvalidValue {
            name: name.to_string(),
            reason,
        })
    };
    if value.is_empty() {
        return refuse("the value is empty");
    }
    if value.len() > MAX_THEME_VALUE_BYTES {
        return refuse("the value is longer than 1024 bytes");
    }
    if let Some(reason) = value.chars().find_map(forbidden_reason) {
        return refuse(reason);
    }
    let mut quote: Option<char> = None;
    let mut depth: usize = 0;
    let mut previous = '\0';
    for c in value.chars() {
        match quote {
            Some(open) if c == open => quote = None,
            Some(_) => {}
            None => match c {
                '"' | '\'' => quote = Some(c),
                '(' => depth += 1,
                ')' if depth == 0 => return refuse("a `)` closes nothing"),
                ')' => depth -= 1,
                '*' if previous == '/' => return refuse("CSS comments are not allowed"),
                '/' if previous == '*' => return refuse("CSS comments are not allowed"),
                _ => {}
            },
        }
        previous = c;
    }
    if quote.is_some() {
        return refuse("a quoted string is not closed");
    }
    if depth != 0 {
        return refuse("a `(` is not closed");
    }
    Ok(())
}

/// Why `c` may not appear anywhere in a value, quoted or not.
fn forbidden_reason(c: char) -> Option<&'static str> {
    match c {
        '<' | '>' => Some("`<` and `>` are not allowed"),
        '{' | '}' => Some("`{` and `}` are not allowed"),
        ';' => Some("`;` is not allowed"),
        '!' => Some("`!` is not allowed (every variable is already `!important`)"),
        '\\' => Some("`\\` (CSS escapes) is not allowed"),
        c if c.is_control() => Some("control characters are not allowed"),
        _ => None,
    }
}

/// RFC 4648 base64 with padding, which CSP hash sources use.
fn base64_standard(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    bytes
        .chunks(3)
        .flat_map(|chunk| {
            let b = [
                chunk[0],
                chunk.get(1).copied().unwrap_or(0),
                chunk.get(2).copied().unwrap_or(0),
            ];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            (0..4).map(move |i| {
                if i > chunk.len() {
                    '='
                } else {
                    char::from(ALPHABET[((n >> (18 - 6 * i)) & 0x3f) as usize])
                }
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn vars(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn nothing_to_add_is_none() {
        assert_eq!(HostHead::new(None, &BTreeMap::new()), Ok(None));
        assert_eq!(HostHead::new(Some("  \n"), &BTreeMap::new()), Ok(None));
    }

    #[test]
    fn variables_become_one_important_root_rule_then_the_html() {
        let head = HostHead::new(
            Some("<link rel=\"stylesheet\" href=\"/app/theme.css\">"),
            &vars(&[
                ("--pico-primary", " light-dark(#7c3aed, #a78bfa) "),
                ("--mbr-sidebar-width", "300px"),
            ]),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            head.html(),
            "<style id=\"mbr-host-theme\">:root { --mbr-sidebar-width: 300px !important; \
             --pico-primary: light-dark(#7c3aed, #a78bfa) !important; }</style>\n\
             <link rel=\"stylesheet\" href=\"/app/theme.css\">"
        );
    }

    #[test]
    fn html_alone_has_no_style_and_no_hash() {
        let head = HostHead::new(Some("<meta name=x>"), &BTreeMap::new())
            .unwrap()
            .unwrap();
        assert_eq!(head.html(), "<meta name=x>");
        assert_eq!(head.style_csp_source(), None);
    }

    #[test]
    fn csp_source_is_the_hash_of_the_style_text() {
        let head = HostHead::new(None, &vars(&[("--a", "red")]))
            .unwrap()
            .unwrap();
        let css = ":root { --a: red !important; }";
        assert!(head.html().contains(css));
        let expected = format!("'sha256-{}'", base64_standard(&Sha256::digest(css)));
        assert_eq!(head.style_csp_source(), Some(expected));
    }

    #[test]
    fn base64_matches_rfc_4648_vectors() {
        let cases = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (input, expected) in cases {
            assert_eq!(base64_standard(input.as_bytes()), expected, "{input}");
        }
    }

    #[test]
    fn accepted_values() {
        for value in [
            "#fff",
            "rgb(1 2 3 / 50%)",
            "light-dark(#fff, oklch(20% 0.02 260))",
            "\"Inter\", 'Helvetica Neue', sans-serif",
            "\"it's\"",
            "calc(100% - var(--x, 2px))",
            "url(\"/app/bg.png\")",
            "\"a ) b ( c\"",
            "0",
        ] {
            assert!(
                HostHead::new(None, &vars(&[("--x", value)])).is_ok(),
                "{value}"
            );
        }
    }

    #[test]
    fn refused_names() {
        for name in [
            "pico-primary",
            "-x",
            "--",
            "--a b",
            "--a:b",
            "--é",
            "--a;}",
            &format!("--{}", "a".repeat(MAX_THEME_NAME_BYTES)),
        ] {
            assert!(
                matches!(
                    HostHead::new(None, &vars(&[(name, "red")])),
                    Err(HostHeadError::InvalidName { .. })
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn refused_values() {
        for value in [
            "",
            "   ",
            "red</style><script>alert(1)</script>",
            "red; } body { display: none",
            "red }",
            "{",
            "red !important",
            "\"unclosed",
            "'unclosed",
            "rgb(1, 2",
            "a)",
            "red /* swallow",
            "*/",
            "\\3c /style",
            "red\nblue",
            "\"</style>\"",
            &"a".repeat(MAX_THEME_VALUE_BYTES + 1),
        ] {
            assert!(
                matches!(
                    HostHead::new(None, &vars(&[("--x", value)])),
                    Err(HostHeadError::InvalidValue { .. })
                ),
                "{value:?}"
            );
        }
    }

    #[test]
    fn the_error_names_the_variable() {
        let error = HostHead::new(None, &vars(&[("--ok", "red"), ("--bad", "x;y")])).unwrap_err();
        assert_eq!(
            error.to_string(),
            "theme variable `--bad`: `;` is not allowed"
        );
    }

    proptest! {
        /// Whatever a host passes, an accepted theme is one `:root` rule
        /// inside one `<style>` element: no value can close the element, the
        /// rule or its own declaration, or open a comment.
        #[test]
        fn an_accepted_theme_never_breaks_out(
            entries in proptest::collection::btree_map(
                "--[a-z]{1,8}",
                "[ -~]{0,40}",
                1..6,
            )
        ) {
            if let Ok(Some(head)) = HostHead::new(None, &entries) {
                let html = head.html();
                let css = html
                    .strip_prefix("<style id=\"mbr-host-theme\">:root { ")
                    .and_then(|rest| rest.strip_suffix(" }</style>"))
                    .expect("one rule in one style element");
                prop_assert!(!css.contains('<'));
                prop_assert!(!css.contains(['{', '}']), "a brace in {css:?}");
                prop_assert!(!css.contains("/*"));
                prop_assert_eq!(css.matches(';').count(), entries.len());
                prop_assert_eq!(css.matches("!important;").count(), entries.len());
            }
        }
    }
}
