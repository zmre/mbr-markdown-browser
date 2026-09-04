//! Turning a filesystem path into the URL that displays it.
//!
//! The inverse of [`crate::path_resolver`]: that module turns a request URL
//! into a filesystem resource, this turns a filesystem path (already known —
//! a CLI argument, a picked file) into the URL a browser should be pointed at.
//! Lived in `main.rs` as a bin-local helper until the GUI's "Open…" picker
//! (`crate::open_picker`, `crate::browser::start_server_for`) needed the same
//! branch: directory vs. media file vs. markdown page vs. plain static file.
//! One copy here, called from both the initial launch in `main.rs` and a live
//! re-point in `browser.rs`, so the two can never resolve the same target to
//! different URLs.

use crate::server::MediaViewerType;
use std::path::Path;

/// Builds a URL path from a relative filesystem path.
///
/// - For directories: returns the path with a trailing slash
/// - For markdown files: replaces the extension with a trailing slash
/// - For other files: returns the path as-is
pub fn build_url_path(
    relative_path: &Path,
    is_directory: bool,
    markdown_extensions: &[String],
) -> String {
    // `path_to_url` keeps the result `/`-separated on Windows, where
    // `to_str()` would hand back `docs\guide.md`.
    let relative_str = crate::url_path::path_to_url(relative_path);

    if is_directory {
        if relative_str.is_empty() {
            String::new()
        } else {
            format!("{}/", relative_str)
        }
    } else {
        replace_markdown_extension_with_slash(&relative_str, markdown_extensions)
    }
}

fn replace_markdown_extension_with_slash(s: &str, extensions: &[String]) -> String {
    if let Some((base, extension)) = s.rsplit_once('.') {
        match extensions
            .iter()
            .find(|cur_ext| extension == cur_ext.as_str())
        {
            Some(_) => format!("{}/", base), // one of the sought extensions is there, replace with a "/"
            None => s.to_string(), // no sought extensions found, just return input as provided
        }
    } else {
        s.to_string() // no extension, so return input as provided
    }
}

/// Builds a media viewer URL for the given media type and file path.
///
/// The returned path is relative to the server root, e.g.,
/// `/.mbr/videos/?path=%2Fvideos%2Fexample.mp4`.
///
/// The `file_url_path` should be the URL path to the file (as returned by
/// [`build_url_path`]), without a leading slash (e.g., `videos/example.mp4`).
pub fn build_media_viewer_url(media_type: MediaViewerType, file_url_path: &str) -> String {
    use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

    // Encode the path for use as a query parameter value.
    // We need to encode everything except unreserved characters.
    const QUERY_ENCODE_SET: &AsciiSet = &CONTROLS
        .add(b' ')
        .add(b'"')
        .add(b'#')
        .add(b'%')
        .add(b'&')
        .add(b'+')
        .add(b'=')
        .add(b'?');

    // Ensure the file path has a leading slash for the query param
    let full_path = if file_url_path.starts_with('/') {
        file_url_path.to_string()
    } else {
        format!("/{file_url_path}")
    };

    let encoded_path = utf8_percent_encode(&full_path, QUERY_ENCODE_SET).to_string();
    format!("{}?path={}", media_type.route_path(), encoded_path)
}

/// Resolves the URL path (no scheme, host or leading slash) that displays
/// `relative_path`, a target already expressed relative to the repository
/// root.
///
/// One function for the branch every GUI launch path used to repeat by hand:
/// a known media extension ([`MediaViewerType::from_path`]) gets its viewer
/// URL, everything else — directories included — goes through
/// [`build_url_path`], which turns a markdown extension into the
/// trailing-slash page URL and leaves a directory or a plain file's path
/// alone. Directories are never checked against `MediaViewerType`, matching
/// the CLI/GUI launch behavior this replaces: `is_directory` short-circuits
/// straight to `build_url_path`.
///
/// Callers join the result onto their own base URL (`http://host:port/`)
/// with [`url::Url::join`] rather than string concatenation: a media viewer
/// URL starts with `/` (an absolute path, replacing the base's path
/// entirely) while a page URL does not (relative to the base), and only
/// `Url::join` treats both correctly.
pub fn resolve_launch_url_path(
    relative_path: &Path,
    is_directory: bool,
    markdown_extensions: &[String],
) -> String {
    if !is_directory && let Some(media_type) = MediaViewerType::from_path(relative_path) {
        let file_url_path = build_url_path(relative_path, false, markdown_extensions);
        return build_media_viewer_url(media_type, &file_url_path);
    }

    build_url_path(relative_path, is_directory, markdown_extensions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_build_url_path_root_directory() {
        let path = Path::new("");
        let extensions = vec!["md".to_string()];
        assert_eq!(build_url_path(path, true, &extensions), "");
    }

    #[test]
    fn test_build_url_path_subdirectory() {
        let path = Path::new("docs/api");
        let extensions = vec!["md".to_string()];
        assert_eq!(build_url_path(path, true, &extensions), "docs/api/");
    }

    #[test]
    fn test_build_url_path_markdown_file() {
        let path = Path::new("readme.md");
        let extensions = vec!["md".to_string()];
        assert_eq!(build_url_path(path, false, &extensions), "readme/");
    }

    #[test]
    fn test_build_url_path_markdown_file_in_subdir() {
        let path = Path::new("docs/guide.md");
        let extensions = vec!["md".to_string()];
        assert_eq!(build_url_path(path, false, &extensions), "docs/guide/");
    }

    #[test]
    fn test_build_url_path_alternate_extension() {
        let path = Path::new("notes.markdown");
        let extensions = vec!["md".to_string(), "markdown".to_string()];
        assert_eq!(build_url_path(path, false, &extensions), "notes/");
    }

    #[test]
    fn test_build_url_path_non_markdown_file() {
        let path = Path::new("image.png");
        let extensions = vec!["md".to_string()];
        assert_eq!(build_url_path(path, false, &extensions), "image.png");
    }

    #[test]
    fn test_replace_markdown_extension_with_slash() {
        let extensions = ["md".to_string()];
        assert_eq!(
            replace_markdown_extension_with_slash("test.md", &extensions),
            "test/"
        );
        assert_eq!(
            replace_markdown_extension_with_slash("test.txt", &extensions),
            "test.txt"
        );
        assert_eq!(
            replace_markdown_extension_with_slash("noext", &extensions),
            "noext"
        );
    }

    #[test]
    fn test_build_media_viewer_url_video() {
        let url = build_media_viewer_url(MediaViewerType::Video, "videos/example.mp4");
        assert_eq!(url, "/.mbr/videos/?path=/videos/example.mp4");
    }

    #[test]
    fn test_build_media_viewer_url_audio() {
        let url = build_media_viewer_url(MediaViewerType::Audio, "music/song.mp3");
        assert_eq!(url, "/.mbr/audio/?path=/music/song.mp3");
    }

    #[test]
    fn test_build_media_viewer_url_image() {
        let url = build_media_viewer_url(MediaViewerType::Image, "images/photo.jpg");
        assert_eq!(url, "/.mbr/images/?path=/images/photo.jpg");
    }

    #[test]
    fn test_build_media_viewer_url_pdf() {
        let url = build_media_viewer_url(MediaViewerType::Pdf, "docs/paper.pdf");
        assert_eq!(url, "/.mbr/pdfs/?path=/docs/paper.pdf");
    }

    #[test]
    fn test_build_media_viewer_url_with_leading_slash() {
        let url = build_media_viewer_url(MediaViewerType::Video, "/videos/example.mp4");
        assert_eq!(url, "/.mbr/videos/?path=/videos/example.mp4");
    }

    #[test]
    fn test_build_media_viewer_url_encodes_spaces() {
        let url = build_media_viewer_url(MediaViewerType::Video, "videos/my video.mp4");
        assert!(url.contains("path=/videos/my%20video.mp4"));
    }

    #[test]
    fn test_build_media_viewer_url_encodes_special_chars() {
        let url = build_media_viewer_url(MediaViewerType::Video, "videos/file#1&2=3.mp4");
        // Hash, ampersand, and equals should be encoded
        assert!(url.contains("path=/videos/file%231%262%3D3.mp4"));
    }

    // ==================== resolve_launch_url_path ====================

    #[test]
    fn test_resolve_launch_url_path_directory() {
        let extensions = vec!["md".to_string()];
        assert_eq!(
            resolve_launch_url_path(Path::new("docs/guide"), true, &extensions),
            "docs/guide/"
        );
    }

    #[test]
    fn test_resolve_launch_url_path_root_directory() {
        let extensions = vec!["md".to_string()];
        assert_eq!(
            resolve_launch_url_path(Path::new(""), true, &extensions),
            ""
        );
    }

    #[test]
    fn test_resolve_launch_url_path_markdown_file() {
        let extensions = vec!["md".to_string()];
        assert_eq!(
            resolve_launch_url_path(Path::new("docs/guide.md"), false, &extensions),
            "docs/guide/"
        );
    }

    #[test]
    fn test_resolve_launch_url_path_video_file() {
        let extensions = vec!["md".to_string()];
        assert_eq!(
            resolve_launch_url_path(Path::new("videos/demo.mp4"), false, &extensions),
            "/.mbr/videos/?path=/videos/demo.mp4"
        );
    }

    #[test]
    fn test_resolve_launch_url_path_pdf_file() {
        let extensions = vec!["md".to_string()];
        assert_eq!(
            resolve_launch_url_path(Path::new("docs/paper.pdf"), false, &extensions),
            "/.mbr/pdfs/?path=/docs/paper.pdf"
        );
    }

    #[test]
    fn test_resolve_launch_url_path_plain_file() {
        let extensions = vec!["md".to_string()];
        assert_eq!(
            resolve_launch_url_path(Path::new("data.csv"), false, &extensions),
            "data.csv"
        );
    }

    /// A directory that happens to share a name with a media extension (an
    /// edge case, but `MediaViewerType::from_path` only looks at the
    /// extension) must still resolve as a directory listing, never a viewer
    /// URL — `is_directory` is checked first and short-circuits.
    #[test]
    fn test_resolve_launch_url_path_directory_short_circuits_media_check() {
        let extensions = vec!["md".to_string()];
        assert_eq!(
            resolve_launch_url_path(Path::new("videos/demo.mp4"), true, &extensions),
            "videos/demo.mp4/"
        );
    }
}
