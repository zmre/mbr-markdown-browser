//! Turning `tao::event::Event::Opened`'s URLs into a filesystem path.
//!
//! macOS delivers a Finder "Open With → MBR" launch (or a document
//! double-click, once `MBR.app` is the default handler) as this event,
//! wrapping `application(_:open:)` — see `browser::InitialLaunch::Deferred`
//! for the launch-ordering guarantee that makes reacting to it race-free.
//!
//! [`first_local_path`] is pure and carries no `cfg`, even though
//! [`crate::browser`] only ever calls it from `#[cfg(target_os = "macos")]`
//! code — `Event::Opened` is never constructed by tao's Windows or Linux
//! backends, so there is nothing to react to there. Keeping the function
//! itself platform-generic means `cargo test` exercises it on every platform
//! this crate is built on, not only the one it actually fires on.

use std::path::PathBuf;

/// The first URL in `urls` that names a local file, if any.
///
/// `application(_:open:)` is not exclusive to document opens — mbr also
/// registers the `mbr://` URL scheme (see `CFBundleURLTypes` in
/// `macos/MBR.app-template/Contents/Info.plist`), and both arrive through the
/// same event. Anything that is not a `file://` URL is silently skipped
/// rather than treated as an error: there is no other scheme registered
/// today that this code needs to understand, and a future one showing up
/// here should degrade to "no usable path" rather than a hard failure.
///
/// The first match wins: mbr opens one window, so a launch naming several
/// files has to pick one, and "the one Finder listed first" is as good a
/// rule as any.
pub(crate) fn first_local_path(urls: &[url::Url]) -> Option<PathBuf> {
    urls.iter().find_map(|url| url.to_file_path().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `file://` URL for `name`, built via `Url::from_file_path` rather
    /// than a hardcoded `file:///tmp/...` literal — `to_file_path`'s parsing
    /// is platform-specific (Windows requires a drive letter; Unix does not),
    /// so a Unix-shaped literal fails to round-trip on Windows and the test
    /// itself is what breaks, not the function under test. `from_file_path`
    /// and `to_file_path` are each other's inverse on whatever platform the
    /// test actually runs on, so this is correct everywhere without `cfg`.
    fn file_url(path: &std::path::Path) -> url::Url {
        url::Url::from_file_path(path).expect("std::env::temp_dir() is always absolute")
    }

    #[test]
    fn finds_the_first_file_url() {
        let path = std::env::temp_dir().join("readme.md");
        let urls = vec![
            url::Url::parse("https://example.com").unwrap(),
            file_url(&path),
        ];
        assert_eq!(first_local_path(&urls), Some(path));
    }

    #[test]
    fn ignores_non_file_urls() {
        let urls = vec![
            url::Url::parse("https://example.com").unwrap(),
            url::Url::parse("mbr://some/deep-link").unwrap(),
        ];
        assert_eq!(first_local_path(&urls), None);
    }

    #[test]
    fn empty_urls_returns_none() {
        assert_eq!(first_local_path(&[]), None);
    }

    #[test]
    fn picks_the_first_of_several_file_urls() {
        let a = std::env::temp_dir().join("a.md");
        let b = std::env::temp_dir().join("b.md");
        let urls = vec![file_url(&a), file_url(&b)];
        assert_eq!(first_local_path(&urls), Some(a));
    }
}
