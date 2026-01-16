//! Embedded KaTeX files for math rendering.
//!
//! This module embeds KaTeX CSS, JavaScript, and font files, providing
//! centralized access to math rendering assets.

/// KaTeX minified CSS
pub const KATEX_CSS: &[u8] = include_bytes!("../templates/katex.0.16.27.min.css");

/// KaTeX minified JavaScript
pub const KATEX_JS: &[u8] = include_bytes!("../templates/katex.0.16.27.min.js");

// Font files - WOFF2 format (primary, smallest)
pub const KATEX_FONT_AMS_REGULAR_WOFF2: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_AMS-Regular.woff2");
pub const KATEX_FONT_CALIGRAPHIC_BOLD_WOFF2: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Caligraphic-Bold.woff2");
pub const KATEX_FONT_CALIGRAPHIC_REGULAR_WOFF2: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Caligraphic-Regular.woff2");
pub const KATEX_FONT_FRAKTUR_BOLD_WOFF2: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Fraktur-Bold.woff2");
pub const KATEX_FONT_FRAKTUR_REGULAR_WOFF2: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Fraktur-Regular.woff2");
pub const KATEX_FONT_MAIN_BOLD_WOFF2: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Main-Bold.woff2");
pub const KATEX_FONT_MAIN_BOLD_ITALIC_WOFF2: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Main-BoldItalic.woff2");
pub const KATEX_FONT_MAIN_ITALIC_WOFF2: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Main-Italic.woff2");
pub const KATEX_FONT_MAIN_REGULAR_WOFF2: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Main-Regular.woff2");
pub const KATEX_FONT_MATH_BOLD_ITALIC_WOFF2: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Math-BoldItalic.woff2");
pub const KATEX_FONT_MATH_ITALIC_WOFF2: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Math-Italic.woff2");
pub const KATEX_FONT_SANS_SERIF_BOLD_WOFF2: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_SansSerif-Bold.woff2");
pub const KATEX_FONT_SANS_SERIF_ITALIC_WOFF2: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_SansSerif-Italic.woff2");
pub const KATEX_FONT_SANS_SERIF_REGULAR_WOFF2: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_SansSerif-Regular.woff2");
pub const KATEX_FONT_SCRIPT_REGULAR_WOFF2: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Script-Regular.woff2");
pub const KATEX_FONT_SIZE1_REGULAR_WOFF2: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Size1-Regular.woff2");
pub const KATEX_FONT_SIZE2_REGULAR_WOFF2: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Size2-Regular.woff2");
pub const KATEX_FONT_SIZE3_REGULAR_WOFF2: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Size3-Regular.woff2");
pub const KATEX_FONT_SIZE4_REGULAR_WOFF2: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Size4-Regular.woff2");
pub const KATEX_FONT_TYPEWRITER_REGULAR_WOFF2: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Typewriter-Regular.woff2");

// Font files - WOFF format (fallback for older browsers)
pub const KATEX_FONT_AMS_REGULAR_WOFF: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_AMS-Regular.woff");
pub const KATEX_FONT_CALIGRAPHIC_BOLD_WOFF: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Caligraphic-Bold.woff");
pub const KATEX_FONT_CALIGRAPHIC_REGULAR_WOFF: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Caligraphic-Regular.woff");
pub const KATEX_FONT_FRAKTUR_BOLD_WOFF: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Fraktur-Bold.woff");
pub const KATEX_FONT_FRAKTUR_REGULAR_WOFF: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Fraktur-Regular.woff");
pub const KATEX_FONT_MAIN_BOLD_WOFF: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Main-Bold.woff");
pub const KATEX_FONT_MAIN_BOLD_ITALIC_WOFF: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Main-BoldItalic.woff");
pub const KATEX_FONT_MAIN_ITALIC_WOFF: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Main-Italic.woff");
pub const KATEX_FONT_MAIN_REGULAR_WOFF: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Main-Regular.woff");
pub const KATEX_FONT_MATH_BOLD_ITALIC_WOFF: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Math-BoldItalic.woff");
pub const KATEX_FONT_MATH_ITALIC_WOFF: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Math-Italic.woff");
pub const KATEX_FONT_SANS_SERIF_BOLD_WOFF: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_SansSerif-Bold.woff");
pub const KATEX_FONT_SANS_SERIF_ITALIC_WOFF: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_SansSerif-Italic.woff");
pub const KATEX_FONT_SANS_SERIF_REGULAR_WOFF: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_SansSerif-Regular.woff");
pub const KATEX_FONT_SCRIPT_REGULAR_WOFF: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Script-Regular.woff");
pub const KATEX_FONT_SIZE1_REGULAR_WOFF: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Size1-Regular.woff");
pub const KATEX_FONT_SIZE2_REGULAR_WOFF: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Size2-Regular.woff");
pub const KATEX_FONT_SIZE3_REGULAR_WOFF: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Size3-Regular.woff");
pub const KATEX_FONT_SIZE4_REGULAR_WOFF: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Size4-Regular.woff");
pub const KATEX_FONT_TYPEWRITER_REGULAR_WOFF: &[u8] =
    include_bytes!("../templates/katex-fonts/KaTeX_Typewriter-Regular.woff");

/// All KaTeX files as (url_path, bytes, mime_type) tuples.
///
/// The url_path is the path without version numbers for cleaner URLs.
pub const KATEX_FILES: &[(&str, &[u8], &str)] = &[
    // Main files
    ("/katex.min.css", KATEX_CSS, "text/css"),
    ("/katex.min.js", KATEX_JS, "application/javascript"),
    // WOFF2 fonts
    (
        "/fonts/KaTeX_AMS-Regular.woff2",
        KATEX_FONT_AMS_REGULAR_WOFF2,
        "font/woff2",
    ),
    (
        "/fonts/KaTeX_Caligraphic-Bold.woff2",
        KATEX_FONT_CALIGRAPHIC_BOLD_WOFF2,
        "font/woff2",
    ),
    (
        "/fonts/KaTeX_Caligraphic-Regular.woff2",
        KATEX_FONT_CALIGRAPHIC_REGULAR_WOFF2,
        "font/woff2",
    ),
    (
        "/fonts/KaTeX_Fraktur-Bold.woff2",
        KATEX_FONT_FRAKTUR_BOLD_WOFF2,
        "font/woff2",
    ),
    (
        "/fonts/KaTeX_Fraktur-Regular.woff2",
        KATEX_FONT_FRAKTUR_REGULAR_WOFF2,
        "font/woff2",
    ),
    (
        "/fonts/KaTeX_Main-Bold.woff2",
        KATEX_FONT_MAIN_BOLD_WOFF2,
        "font/woff2",
    ),
    (
        "/fonts/KaTeX_Main-BoldItalic.woff2",
        KATEX_FONT_MAIN_BOLD_ITALIC_WOFF2,
        "font/woff2",
    ),
    (
        "/fonts/KaTeX_Main-Italic.woff2",
        KATEX_FONT_MAIN_ITALIC_WOFF2,
        "font/woff2",
    ),
    (
        "/fonts/KaTeX_Main-Regular.woff2",
        KATEX_FONT_MAIN_REGULAR_WOFF2,
        "font/woff2",
    ),
    (
        "/fonts/KaTeX_Math-BoldItalic.woff2",
        KATEX_FONT_MATH_BOLD_ITALIC_WOFF2,
        "font/woff2",
    ),
    (
        "/fonts/KaTeX_Math-Italic.woff2",
        KATEX_FONT_MATH_ITALIC_WOFF2,
        "font/woff2",
    ),
    (
        "/fonts/KaTeX_SansSerif-Bold.woff2",
        KATEX_FONT_SANS_SERIF_BOLD_WOFF2,
        "font/woff2",
    ),
    (
        "/fonts/KaTeX_SansSerif-Italic.woff2",
        KATEX_FONT_SANS_SERIF_ITALIC_WOFF2,
        "font/woff2",
    ),
    (
        "/fonts/KaTeX_SansSerif-Regular.woff2",
        KATEX_FONT_SANS_SERIF_REGULAR_WOFF2,
        "font/woff2",
    ),
    (
        "/fonts/KaTeX_Script-Regular.woff2",
        KATEX_FONT_SCRIPT_REGULAR_WOFF2,
        "font/woff2",
    ),
    (
        "/fonts/KaTeX_Size1-Regular.woff2",
        KATEX_FONT_SIZE1_REGULAR_WOFF2,
        "font/woff2",
    ),
    (
        "/fonts/KaTeX_Size2-Regular.woff2",
        KATEX_FONT_SIZE2_REGULAR_WOFF2,
        "font/woff2",
    ),
    (
        "/fonts/KaTeX_Size3-Regular.woff2",
        KATEX_FONT_SIZE3_REGULAR_WOFF2,
        "font/woff2",
    ),
    (
        "/fonts/KaTeX_Size4-Regular.woff2",
        KATEX_FONT_SIZE4_REGULAR_WOFF2,
        "font/woff2",
    ),
    (
        "/fonts/KaTeX_Typewriter-Regular.woff2",
        KATEX_FONT_TYPEWRITER_REGULAR_WOFF2,
        "font/woff2",
    ),
    // WOFF fonts (fallback)
    (
        "/fonts/KaTeX_AMS-Regular.woff",
        KATEX_FONT_AMS_REGULAR_WOFF,
        "font/woff",
    ),
    (
        "/fonts/KaTeX_Caligraphic-Bold.woff",
        KATEX_FONT_CALIGRAPHIC_BOLD_WOFF,
        "font/woff",
    ),
    (
        "/fonts/KaTeX_Caligraphic-Regular.woff",
        KATEX_FONT_CALIGRAPHIC_REGULAR_WOFF,
        "font/woff",
    ),
    (
        "/fonts/KaTeX_Fraktur-Bold.woff",
        KATEX_FONT_FRAKTUR_BOLD_WOFF,
        "font/woff",
    ),
    (
        "/fonts/KaTeX_Fraktur-Regular.woff",
        KATEX_FONT_FRAKTUR_REGULAR_WOFF,
        "font/woff",
    ),
    (
        "/fonts/KaTeX_Main-Bold.woff",
        KATEX_FONT_MAIN_BOLD_WOFF,
        "font/woff",
    ),
    (
        "/fonts/KaTeX_Main-BoldItalic.woff",
        KATEX_FONT_MAIN_BOLD_ITALIC_WOFF,
        "font/woff",
    ),
    (
        "/fonts/KaTeX_Main-Italic.woff",
        KATEX_FONT_MAIN_ITALIC_WOFF,
        "font/woff",
    ),
    (
        "/fonts/KaTeX_Main-Regular.woff",
        KATEX_FONT_MAIN_REGULAR_WOFF,
        "font/woff",
    ),
    (
        "/fonts/KaTeX_Math-BoldItalic.woff",
        KATEX_FONT_MATH_BOLD_ITALIC_WOFF,
        "font/woff",
    ),
    (
        "/fonts/KaTeX_Math-Italic.woff",
        KATEX_FONT_MATH_ITALIC_WOFF,
        "font/woff",
    ),
    (
        "/fonts/KaTeX_SansSerif-Bold.woff",
        KATEX_FONT_SANS_SERIF_BOLD_WOFF,
        "font/woff",
    ),
    (
        "/fonts/KaTeX_SansSerif-Italic.woff",
        KATEX_FONT_SANS_SERIF_ITALIC_WOFF,
        "font/woff",
    ),
    (
        "/fonts/KaTeX_SansSerif-Regular.woff",
        KATEX_FONT_SANS_SERIF_REGULAR_WOFF,
        "font/woff",
    ),
    (
        "/fonts/KaTeX_Script-Regular.woff",
        KATEX_FONT_SCRIPT_REGULAR_WOFF,
        "font/woff",
    ),
    (
        "/fonts/KaTeX_Size1-Regular.woff",
        KATEX_FONT_SIZE1_REGULAR_WOFF,
        "font/woff",
    ),
    (
        "/fonts/KaTeX_Size2-Regular.woff",
        KATEX_FONT_SIZE2_REGULAR_WOFF,
        "font/woff",
    ),
    (
        "/fonts/KaTeX_Size3-Regular.woff",
        KATEX_FONT_SIZE3_REGULAR_WOFF,
        "font/woff",
    ),
    (
        "/fonts/KaTeX_Size4-Regular.woff",
        KATEX_FONT_SIZE4_REGULAR_WOFF,
        "font/woff",
    ),
    (
        "/fonts/KaTeX_Typewriter-Regular.woff",
        KATEX_FONT_TYPEWRITER_REGULAR_WOFF,
        "font/woff",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_katex_files_not_empty() {
        for (path, content, _mime) in KATEX_FILES.iter() {
            assert!(
                !content.is_empty(),
                "KaTeX file {} should not be empty",
                path
            );
        }
    }

    #[test]
    fn test_katex_file_count() {
        // 2 main files (CSS, JS) + 20 woff2 fonts + 20 woff fonts = 42 total
        assert_eq!(KATEX_FILES.len(), 42);
    }

    #[test]
    fn test_core_files_present() {
        assert!(
            KATEX_FILES
                .iter()
                .any(|(path, _, _)| *path == "/katex.min.css"),
            "KaTeX CSS should be present"
        );
        assert!(
            KATEX_FILES
                .iter()
                .any(|(path, _, _)| *path == "/katex.min.js"),
            "KaTeX JS should be present"
        );
    }

    #[test]
    fn test_mime_types_correct() {
        for (path, _, mime) in KATEX_FILES.iter() {
            if path.ends_with(".css") {
                assert_eq!(
                    *mime, "text/css",
                    "CSS files should have text/css mime type"
                );
            } else if path.ends_with(".js") {
                assert_eq!(
                    *mime, "application/javascript",
                    "JS files should have application/javascript mime type"
                );
            } else if path.ends_with(".woff2") {
                assert_eq!(
                    *mime, "font/woff2",
                    "WOFF2 files should have font/woff2 mime type"
                );
            } else if path.ends_with(".woff") {
                assert_eq!(
                    *mime, "font/woff",
                    "WOFF files should have font/woff mime type"
                );
            }
        }
    }
}
