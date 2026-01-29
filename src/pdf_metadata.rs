//! PDF metadata extraction and cover image generation.
//!
//! Provides functionality to extract metadata and generate cover images
//! from PDF files using lopdf (for metadata) and pdfium-render (for rendering).

use crate::errors::PdfMetadataError;
use std::path::Path;
#[cfg(feature = "media-metadata")]
use std::path::PathBuf;

/// Metadata extracted from a PDF file.
#[derive(Debug, Clone, Default)]
pub struct PdfMetadata {
    /// Document title from Info dictionary
    pub title: Option<String>,
    /// Document author from Info dictionary
    pub author: Option<String>,
    /// Document subject from Info dictionary
    pub subject: Option<String>,
    /// Keywords (comma-separated in PDF, parsed to Vec)
    pub keywords: Option<Vec<String>>,
    /// Total number of pages
    pub num_pages: u32,
}

/// Probe a PDF file to extract metadata without rendering.
pub fn probe_pdf(path: &Path) -> Result<PdfMetadata, PdfMetadataError> {
    let doc = lopdf::Document::load(path).map_err(|e| PdfMetadataError::OpenFailed {
        path: path.to_path_buf(),
        source: e,
    })?;

    let num_pages = doc.get_pages().len() as u32;

    // Get Info dictionary reference from trailer
    let info = doc
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|obj| obj.as_reference().ok())
        .and_then(|info_ref| doc.get_dictionary(info_ref).ok());

    let (title, author, subject, keywords) = if let Some(info) = info {
        (
            get_string_from_dict(&doc, info, b"Title"),
            get_string_from_dict(&doc, info, b"Author"),
            get_string_from_dict(&doc, info, b"Subject"),
            get_string_from_dict(&doc, info, b"Keywords").map(|s| {
                s.split(',')
                    .map(|k| k.trim().to_string())
                    .filter(|k| !k.is_empty())
                    .collect()
            }),
        )
    } else {
        (None, None, None, None)
    };

    Ok(PdfMetadata {
        title,
        author,
        subject,
        keywords,
        num_pages,
    })
}

/// Extract a string value from a PDF dictionary, handling various encodings.
fn get_string_from_dict(
    doc: &lopdf::Document,
    dict: &lopdf::Dictionary,
    key: &[u8],
) -> Option<String> {
    let obj = dict.get(key).ok()?;
    pdf_object_to_string(doc, obj)
}

/// Convert a PDF object to a string, resolving references and handling encodings.
fn pdf_object_to_string(doc: &lopdf::Document, obj: &lopdf::Object) -> Option<String> {
    match obj {
        lopdf::Object::String(bytes, _) => decode_pdf_string(bytes),
        lopdf::Object::Reference(r) => doc
            .get_object(*r)
            .ok()
            .and_then(|o| pdf_object_to_string(doc, o)),
        _ => None,
    }
}

/// Decode a PDF string, handling UTF-16BE BOM and Latin-1 fallback.
fn decode_pdf_string(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }

    // Check for UTF-16BE BOM (0xFE 0xFF)
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        // UTF-16BE encoded
        let utf16: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16(&utf16).ok()
    } else {
        // Try UTF-8 first, then fall back to Latin-1
        String::from_utf8(bytes.to_vec())
            .ok()
            .or_else(|| Some(bytes.iter().map(|&b| b as char).collect()))
    }
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
}

/// Parse a request path to check if it's a PDF cover image request.
///
/// Returns the base PDF path (without .cover.png suffix) if valid.
///
/// # Examples
/// ```ignore
/// assert_eq!(parse_pdf_cover_request("docs/report.pdf.cover.png"), Some("docs/report.pdf"));
/// assert_eq!(parse_pdf_cover_request("docs/report.pdf"), None);
/// ```
pub fn parse_pdf_cover_request(path: &str) -> Option<&str> {
    let base = path.strip_suffix(".cover.png")?;
    // Verify the base ends with .pdf (case-insensitive)
    if base.to_lowercase().ends_with(".pdf") {
        Some(base)
    } else {
        None
    }
}

/// Extract the first page of a PDF as a PNG image.
///
/// Uses pdfium-render to render the first page at a reasonable resolution
/// (max width 1200px, preserving aspect ratio).
#[cfg(feature = "media-metadata")]
pub fn extract_cover(path: &Path) -> Result<Vec<u8>, PdfMetadataError> {
    use image::ImageFormat;
    use pdfium_render::prelude::*;

    // Initialize pdfium - try environment variable first, then system library
    let pdfium = create_pdfium_instance()?;

    // Load the PDF document (no password)
    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| map_pdfium_error(e, path.to_path_buf()))?;

    // Get first page
    let page = document
        .pages()
        .first()
        .map_err(|_| PdfMetadataError::NoPages {
            path: path.to_path_buf(),
        })?;

    // Calculate render dimensions - max 1200px width, preserve aspect ratio
    let page_width = page.width().value;
    let page_height = page.height().value;
    let scale = if page_width > 1200.0 {
        1200.0 / page_width
    } else {
        1.0
    };
    let render_width = (page_width * scale) as i32;
    let max_height = (page_height * scale).max(1600.0) as i32;

    // Render to bitmap
    let config = PdfRenderConfig::new()
        .set_target_width(render_width)
        .set_maximum_height(max_height);

    let bitmap = page
        .render_with_config(&config)
        .map_err(|e| PdfMetadataError::RenderFailed(format!("Render failed: {}", e)))?;

    // Convert to image and encode as PNG
    let image = bitmap.as_image();
    let mut png_bytes = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut png_bytes), ImageFormat::Png)
        .map_err(|e| PdfMetadataError::EncodeFailed(format!("PNG encode failed: {}", e)))?;

    Ok(png_bytes)
}

/// Create a Pdfium instance, trying PDFIUM_DYNAMIC_LIB_PATH env var first,
/// then falling back to system library search.
#[cfg(feature = "media-metadata")]
fn create_pdfium_instance() -> Result<pdfium_render::prelude::Pdfium, PdfMetadataError> {
    use pdfium_render::prelude::Pdfium;

    // Try PDFIUM_DYNAMIC_LIB_PATH environment variable first
    if let Ok(lib_path) = std::env::var("PDFIUM_DYNAMIC_LIB_PATH") {
        // Try .dylib first (macOS)
        let lib_file = PathBuf::from(&lib_path).join("libpdfium.dylib");
        if lib_file.exists() {
            tracing::debug!("Attempting to load pdfium from: {:?}", lib_file);
            match Pdfium::bind_to_library(&lib_file) {
                Ok(bindings) => {
                    tracing::debug!("Successfully loaded pdfium from {:?}", lib_file);
                    return Ok(Pdfium::new(bindings));
                }
                Err(e) => {
                    tracing::warn!("Failed to bind to pdfium at {:?}: {}", lib_file, e);
                    // Return early with the specific error from the explicit path
                    return Err(PdfMetadataError::RenderFailed(format!(
                        "Failed to load pdfium from {:?}: {}",
                        lib_file, e
                    )));
                }
            }
        }
        // Try .so (Linux)
        let lib_file_so = PathBuf::from(&lib_path).join("libpdfium.so");
        if lib_file_so.exists() {
            tracing::debug!("Attempting to load pdfium from: {:?}", lib_file_so);
            match Pdfium::bind_to_library(&lib_file_so) {
                Ok(bindings) => {
                    tracing::debug!("Successfully loaded pdfium from {:?}", lib_file_so);
                    return Ok(Pdfium::new(bindings));
                }
                Err(e) => {
                    tracing::warn!("Failed to bind to pdfium at {:?}: {}", lib_file_so, e);
                    // Return early with the specific error from the explicit path
                    return Err(PdfMetadataError::RenderFailed(format!(
                        "Failed to load pdfium from {:?}: {}",
                        lib_file_so, e
                    )));
                }
            }
        }
    }

    // Fall back to system library
    tracing::debug!("Attempting to load pdfium from system library");
    match Pdfium::bind_to_system_library() {
        Ok(bindings) => {
            tracing::debug!("Successfully loaded pdfium from system library");
            Ok(Pdfium::new(bindings))
        }
        Err(e) => {
            tracing::warn!("Failed to load pdfium from system library: {}", e);
            Err(PdfMetadataError::RenderFailed(format!(
                "Failed to initialize pdfium library: {}",
                e
            )))
        }
    }
}

/// Map a pdfium-render error to our PdfMetadataError type.
#[cfg(feature = "media-metadata")]
fn map_pdfium_error(e: pdfium_render::prelude::PdfiumError, path: PathBuf) -> PdfMetadataError {
    use pdfium_render::prelude::{PdfiumError, PdfiumInternalError};

    match e {
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) => {
            PdfMetadataError::PasswordProtected { path }
        }
        _ => PdfMetadataError::RenderFailed(format!("Failed to load PDF: {}", e)),
    }
}

/// Extract the cover image of a PDF and save it as a sidecar file.
///
/// Creates a file named `{path}.cover.png` next to the PDF file.
/// Returns the path to the created sidecar file on success.
///
/// # Examples
/// ```ignore
/// let sidecar = save_cover(Path::new("docs/report.pdf"))?;
/// assert_eq!(sidecar, PathBuf::from("docs/report.pdf.cover.png"));
/// ```
#[cfg(feature = "media-metadata")]
pub fn save_cover(path: &Path) -> Result<PathBuf, PdfMetadataError> {
    let png_bytes = extract_cover(path)?;

    let sidecar_path = PathBuf::from(format!("{}.cover.png", path.display()));

    std::fs::write(&sidecar_path, png_bytes)?;

    Ok(sidecar_path)
}

/// Result of processing multiple PDFs for cover extraction.
#[cfg(feature = "media-metadata")]
#[derive(Debug, Default)]
pub struct ExtractCoversResult {
    /// Number of covers successfully created
    pub success_count: usize,
    /// Number of PDFs that failed to process
    pub failure_count: usize,
    /// Details about failed extractions (path, error message)
    pub failures: Vec<(PathBuf, String)>,
}

/// Recursively extract cover images from all PDF files in a directory.
///
/// Walks the directory tree, finds all `.pdf` files (case-insensitive),
/// and creates `.cover.png` sidecar files for each.
///
/// # Arguments
/// * `dir` - Path to the directory to process
/// * `progress_callback` - Optional callback for progress reporting, called with (pdf_path, sidecar_path)
///
/// # Returns
/// An `ExtractCoversResult` containing success/failure counts and error details.
#[cfg(feature = "media-metadata")]
pub fn extract_pdf_covers_recursive<F>(dir: &Path, mut progress_callback: F) -> ExtractCoversResult
where
    F: FnMut(&Path, Option<&Path>),
{
    use walkdir::WalkDir;

    let mut result = ExtractCoversResult::default();

    for entry in WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        // Check if it's a PDF file (case-insensitive)
        if path.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
        {
            match save_cover(path) {
                Ok(sidecar_path) => {
                    progress_callback(path, Some(&sidecar_path));
                    result.success_count += 1;
                }
                Err(e) => {
                    progress_callback(path, None);
                    result.failures.push((path.to_path_buf(), e.to_string()));
                    result.failure_count += 1;
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_pdfs_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/pdfs")
    }

    // ==================== parse_pdf_cover_request tests ====================

    #[test]
    fn test_parse_pdf_cover_request_valid() {
        assert_eq!(
            parse_pdf_cover_request("docs/report.pdf.cover.png"),
            Some("docs/report.pdf")
        );
        assert_eq!(
            parse_pdf_cover_request("docs/REPORT.PDF.cover.png"),
            Some("docs/REPORT.PDF")
        );
        assert_eq!(parse_pdf_cover_request("a.pdf.cover.png"), Some("a.pdf"));
        // Test with spaces in path
        assert_eq!(
            parse_pdf_cover_request("some dir/my file.pdf.cover.png"),
            Some("some dir/my file.pdf")
        );
    }

    #[test]
    fn test_parse_pdf_cover_request_invalid() {
        assert_eq!(parse_pdf_cover_request("docs/report.pdf"), None);
        assert_eq!(parse_pdf_cover_request("docs/report.png"), None);
        assert_eq!(parse_pdf_cover_request("docs/report.txt.cover.png"), None);
        assert_eq!(parse_pdf_cover_request("docs/report.cover.png"), None);
        assert_eq!(parse_pdf_cover_request("docs/report.pdf.cover.jpg"), None);
        assert_eq!(parse_pdf_cover_request(""), None);
    }

    // ==================== extract_cover tests ====================

    #[cfg(feature = "media-metadata")]
    #[test]
    fn test_extract_cover_success() {
        let path = test_pdfs_dir().join("DGA.pdf");
        let result = extract_cover(&path);

        match result {
            Ok(bytes) => {
                // Verify PNG magic bytes
                assert!(bytes.len() > 8, "PNG should have at least 8 bytes");
                assert_eq!(
                    &bytes[0..4],
                    &[0x89, 0x50, 0x4E, 0x47],
                    "Should be valid PNG"
                );
            }
            Err(e) => {
                // If pdfium library not available, skip test
                if e.to_string().contains("library")
                    || e.to_string().contains("not found")
                    || e.to_string().contains("bind")
                {
                    eprintln!("Skipping test: pdfium library not available: {}", e);
                    return;
                }
                panic!("Unexpected error: {}", e);
            }
        }
    }

    #[cfg(feature = "media-metadata")]
    #[test]
    fn test_extract_cover_nonexistent() {
        let path = test_pdfs_dir().join("nonexistent.pdf");
        let result = extract_cover(&path);
        assert!(result.is_err(), "Should fail for nonexistent file");
    }

    // ==================== probe_pdf tests ====================

    #[test]
    fn test_probe_dga_pdf() {
        let path = test_pdfs_dir().join("DGA.pdf");
        let meta = probe_pdf(&path).expect("Should parse DGA.pdf");

        assert!(meta.title.is_some(), "DGA.pdf should have title");
        assert!(
            meta.title.as_ref().unwrap().contains("Dietary Guidelines"),
            "Title should contain 'Dietary Guidelines', got: {:?}",
            meta.title
        );
        assert!(meta.num_pages > 0, "Should have pages");
    }

    #[test]
    fn test_probe_united_nations_charter_pdf() {
        let path = test_pdfs_dir().join("united_nations_charter.pdf");
        let meta = probe_pdf(&path).expect("Should parse united_nations_charter.pdf");

        assert!(meta.title.is_some(), "Should have title");
        assert!(
            meta.title
                .as_ref()
                .unwrap()
                .contains("United Nations Charter"),
            "Title should contain 'United Nations Charter', got: {:?}",
            meta.title
        );
        assert_eq!(
            meta.author.as_deref(),
            Some("Richard Mazula"),
            "Author should be Richard Mazula"
        );
        assert_eq!(
            meta.subject.as_deref(),
            Some("freedom"),
            "Subject should be freedom"
        );
    }

    #[test]
    fn test_probe_f1099_pdf() {
        let path = test_pdfs_dir().join("f1099msc--dft.pdf");
        let meta = probe_pdf(&path).expect("Should parse f1099msc--dft.pdf");

        assert!(meta.title.is_some(), "Should have title");
        assert!(
            meta.title.as_ref().unwrap().contains("1099"),
            "Title should contain '1099', got: {:?}",
            meta.title
        );
        assert!(meta.author.is_some(), "Should have author");
        assert!(meta.subject.is_some(), "Should have subject");
    }

    #[test]
    fn test_probe_nonexistent_pdf() {
        let path = test_pdfs_dir().join("nonexistent.pdf");
        let result = probe_pdf(&path);
        assert!(result.is_err(), "Should fail for nonexistent file");
    }

    #[test]
    fn test_decode_pdf_string_empty() {
        assert_eq!(decode_pdf_string(&[]), None);
    }

    #[test]
    fn test_decode_pdf_string_utf8() {
        let bytes = b"Hello World";
        assert_eq!(decode_pdf_string(bytes), Some("Hello World".to_string()));
    }

    #[test]
    fn test_decode_pdf_string_utf16be() {
        // UTF-16BE BOM followed by "Hi"
        let bytes = [0xFE, 0xFF, 0x00, 0x48, 0x00, 0x69];
        assert_eq!(decode_pdf_string(&bytes), Some("Hi".to_string()));
    }

    #[test]
    fn test_decode_pdf_string_trims_whitespace() {
        let bytes = b"  Hello  ";
        assert_eq!(decode_pdf_string(bytes), Some("Hello".to_string()));
    }

    #[test]
    fn test_decode_pdf_string_filters_empty() {
        let bytes = b"   ";
        assert_eq!(decode_pdf_string(bytes), None);
    }

    // ==================== save_cover tests ====================

    #[cfg(feature = "media-metadata")]
    #[test]
    fn test_save_cover_creates_sidecar_file() {
        let test_dir = tempfile::tempdir().expect("create temp dir");
        let pdf_path = test_pdfs_dir().join("DGA.pdf");

        // Copy PDF to temp dir for testing
        let temp_pdf = test_dir.path().join("test.pdf");
        std::fs::copy(&pdf_path, &temp_pdf).expect("copy pdf");

        let result = super::save_cover(&temp_pdf);

        match result {
            Ok(sidecar_path) => {
                // Verify sidecar path is correct
                assert_eq!(
                    sidecar_path,
                    test_dir.path().join("test.pdf.cover.png"),
                    "Sidecar path should be {{pdf}}.cover.png"
                );

                // Verify file exists
                assert!(sidecar_path.exists(), "Sidecar file should exist");

                // Verify it's a valid PNG
                let bytes = std::fs::read(&sidecar_path).expect("read sidecar");
                assert!(bytes.len() > 8, "PNG should have at least 8 bytes");
                assert_eq!(
                    &bytes[0..4],
                    &[0x89, 0x50, 0x4E, 0x47],
                    "Should be valid PNG"
                );
            }
            Err(e) => {
                // If pdfium library not available, skip test
                if e.to_string().contains("library")
                    || e.to_string().contains("not found")
                    || e.to_string().contains("bind")
                {
                    eprintln!("Skipping test: pdfium library not available: {}", e);
                    return;
                }
                panic!("Unexpected error: {}", e);
            }
        }
    }

    #[cfg(feature = "media-metadata")]
    #[test]
    fn test_save_cover_nonexistent_file() {
        let path = PathBuf::from("/nonexistent/path/to/file.pdf");
        let result = super::save_cover(&path);
        assert!(result.is_err(), "Should fail for nonexistent file");
    }

    // ==================== extract_pdf_covers_recursive tests ====================

    #[cfg(feature = "media-metadata")]
    #[test]
    fn test_extract_pdf_covers_recursive_empty_dir() {
        let test_dir = tempfile::tempdir().expect("create temp dir");

        let result = super::extract_pdf_covers_recursive(test_dir.path(), |_, _| {});

        assert_eq!(result.success_count, 0);
        assert_eq!(result.failure_count, 0);
        assert!(result.failures.is_empty());
    }

    #[cfg(feature = "media-metadata")]
    #[test]
    fn test_extract_pdf_covers_recursive_with_pdfs() {
        let test_dir = tempfile::tempdir().expect("create temp dir");
        let pdf_path = test_pdfs_dir().join("DGA.pdf");

        // Copy PDF to temp dir
        let temp_pdf = test_dir.path().join("test.pdf");
        std::fs::copy(&pdf_path, &temp_pdf).expect("copy pdf");

        // Create a subdirectory with another PDF
        let subdir = test_dir.path().join("subdir");
        std::fs::create_dir(&subdir).expect("create subdir");
        let temp_pdf2 = subdir.join("test2.pdf");
        std::fs::copy(&pdf_path, &temp_pdf2).expect("copy pdf to subdir");

        let mut progress_calls = 0;
        let result = super::extract_pdf_covers_recursive(test_dir.path(), |_, sidecar| {
            if sidecar.is_some() {
                progress_calls += 1;
            }
        });

        // Check if pdfium is available - if not, all will fail
        if result.success_count == 0 && result.failure_count > 0 {
            let first_error = &result.failures[0].1;
            if first_error.contains("library")
                || first_error.contains("not found")
                || first_error.contains("bind")
            {
                eprintln!("Skipping test: pdfium library not available");
                return;
            }
        }

        assert_eq!(result.success_count, 2, "Should process both PDFs");
        assert_eq!(result.failure_count, 0, "No failures expected");
        assert_eq!(
            progress_calls, 2,
            "Progress callback should be called twice"
        );

        // Verify sidecar files exist
        assert!(
            test_dir.path().join("test.pdf.cover.png").exists(),
            "First sidecar should exist"
        );
        assert!(
            subdir.join("test2.pdf.cover.png").exists(),
            "Second sidecar should exist"
        );
    }

    #[cfg(feature = "media-metadata")]
    #[test]
    fn test_extract_pdf_covers_recursive_case_insensitive() {
        let test_dir = tempfile::tempdir().expect("create temp dir");
        let pdf_path = test_pdfs_dir().join("DGA.pdf");

        // Copy PDF with uppercase extension
        let temp_pdf = test_dir.path().join("test.PDF");
        std::fs::copy(&pdf_path, &temp_pdf).expect("copy pdf");

        let result = super::extract_pdf_covers_recursive(test_dir.path(), |_, _| {});

        // Check if pdfium is available
        if result.success_count == 0 && result.failure_count > 0 {
            let first_error = &result.failures[0].1;
            if first_error.contains("library")
                || first_error.contains("not found")
                || first_error.contains("bind")
            {
                eprintln!("Skipping test: pdfium library not available");
                return;
            }
        }

        assert_eq!(
            result.success_count, 1,
            "Should process PDF with uppercase extension"
        );
    }

    #[cfg(feature = "media-metadata")]
    #[test]
    fn test_extract_covers_result_default() {
        let result = super::ExtractCoversResult::default();
        assert_eq!(result.success_count, 0);
        assert_eq!(result.failure_count, 0);
        assert!(result.failures.is_empty());
    }

    // ==================== Property-based tests ====================

    use proptest::prelude::*;

    // Strategy for valid path components (no special characters that would break paths)
    fn path_component_strategy() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_-]{1,15}"
    }

    // Strategy for valid PDF base names (without extension)
    fn pdf_basename_strategy() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_-]{1,20}"
    }

    proptest! {
        /// parse_pdf_cover_request is deterministic - same input always gives same output
        #[test]
        fn prop_parse_pdf_cover_deterministic(path in ".*") {
            let result1 = parse_pdf_cover_request(&path);
            let result2 = parse_pdf_cover_request(&path);
            prop_assert_eq!(result1, result2);
        }

        /// Valid PDF cover requests always return Some with base path
        #[test]
        fn prop_valid_pdf_cover_request_returns_base(
            dir in proptest::option::of(path_component_strategy()),
            name in pdf_basename_strategy(),
            ext_case in prop_oneof!["pdf", "PDF", "Pdf"]
        ) {
            let base = match dir {
                Some(d) => format!("{}/{}.{}", d, name, ext_case),
                None => format!("{}.{}", name, ext_case),
            };
            let full_path = format!("{}.cover.png", base);

            let result = parse_pdf_cover_request(&full_path);
            prop_assert!(result.is_some(), "Valid PDF cover request should return Some");
            prop_assert_eq!(result.unwrap(), base.as_str());
        }

        /// Paths not ending in .cover.png always return None
        #[test]
        fn prop_non_cover_suffix_returns_none(
            path in "[a-zA-Z0-9_/.\\-]{1,50}",
            suffix in prop_oneof![".png", ".jpg", ".pdf", ".cover.jpg", ""]
        ) {
            // Skip if it accidentally ends with .cover.png
            let full_path = format!("{}{}", path, suffix);
            if full_path.ends_with(".cover.png") {
                return Ok(());
            }
            let result = parse_pdf_cover_request(&full_path);
            prop_assert!(result.is_none(), "Non .cover.png paths should return None");
        }

        /// Paths with .cover.png but not .pdf before it return None
        #[test]
        fn prop_non_pdf_cover_returns_none(
            name in pdf_basename_strategy(),
            ext in prop_oneof!["txt", "doc", "jpg", "png", ""]
        ) {
            let path = if ext.is_empty() {
                format!("{}.cover.png", name)
            } else {
                format!("{}.{}.cover.png", name, ext)
            };

            let result = parse_pdf_cover_request(&path);
            prop_assert!(result.is_none(), "Non-PDF .cover.png should return None: {}", path);
        }

        /// The returned base path when stripped from input leaves only .cover.png
        #[test]
        fn prop_base_path_plus_suffix_equals_input(
            dir in proptest::option::of(path_component_strategy()),
            name in pdf_basename_strategy()
        ) {
            let base = match dir {
                Some(d) => format!("{}/{}.pdf", d, name),
                None => format!("{}.pdf", name),
            };
            let full_path = format!("{}.cover.png", base);

            if let Some(result_base) = parse_pdf_cover_request(&full_path) {
                let reconstructed = format!("{}.cover.png", result_base);
                prop_assert_eq!(reconstructed, full_path);
            }
        }

        /// PDF extension case variations all work
        #[test]
        fn prop_pdf_extension_case_insensitive(
            name in pdf_basename_strategy()
        ) {
            let variations = [
                format!("{}.pdf.cover.png", name),
                format!("{}.PDF.cover.png", name),
                format!("{}.Pdf.cover.png", name),
                format!("{}.pDf.cover.png", name),
            ];

            for path in &variations {
                let result = parse_pdf_cover_request(path);
                prop_assert!(result.is_some(), "Should parse: {}", path);
            }
        }
    }
}
