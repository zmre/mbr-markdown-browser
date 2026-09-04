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

    #[test]
    fn finds_the_first_file_url() {
        let urls = vec![
            url::Url::parse("https://example.com").unwrap(),
            url::Url::parse("file:///tmp/notes/readme.md").unwrap(),
        ];
        assert_eq!(
            first_local_path(&urls),
            Some(PathBuf::from("/tmp/notes/readme.md"))
        );
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
        let urls = vec![
            url::Url::parse("file:///tmp/a.md").unwrap(),
            url::Url::parse("file:///tmp/b.md").unwrap(),
        ];
        assert_eq!(first_local_path(&urls), Some(PathBuf::from("/tmp/a.md")));
    }
}
