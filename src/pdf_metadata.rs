//! PDF metadata extraction and cover image generation.
//!
//! Provides functionality to extract metadata and generate cover images
//! from PDF files using lopdf (for metadata) and pdfium-render (for rendering).
//!
//! # Thread Safety
//!
//! PDFium is a C++ library with global state that is NOT thread-safe for
//! concurrent document operations. We use a semaphore to limit concurrent
//! pdfium operations to prevent segfaults when many PDF cover requests
//! arrive simultaneously.

use crate::errors::PdfMetadataError;
use std::path::Path;
#[cfg(feature = "media-metadata")]
use std::path::PathBuf;
#[cfg(feature = "media-metadata")]
use std::sync::OnceLock;
#[cfg(feature = "media-metadata")]
use tokio::sync::Semaphore;

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
/// Returns the base PDF path (without .cover.jpg suffix) if valid.
///
/// # Examples
/// ```ignore
/// assert_eq!(parse_pdf_cover_request("docs/report.pdf.cover.jpg"), Some("docs/report.pdf"));
/// assert_eq!(parse_pdf_cover_request("docs/report.pdf"), None);
/// ```
pub fn parse_pdf_cover_request(path: &str) -> Option<&str> {
    let base = path.strip_suffix(".cover.jpg")?;
    // Verify the base ends with .pdf (case-insensitive)
    if base.to_lowercase().ends_with(".pdf") {
        Some(base)
    } else {
        None
    }
}

/// Global semaphore to limit concurrent pdfium operations.
///
/// PDFium is not thread-safe for concurrent document operations from
/// multiple library instances. Limiting to 1 permit ensures only one
/// PDF is being rendered at a time, preventing segfaults.
#[cfg(feature = "media-metadata")]
static PDFIUM_SEMAPHORE: OnceLock<Semaphore> = OnceLock::new();

/// Get or initialize the pdfium semaphore.
#[cfg(feature = "media-metadata")]
fn pdfium_semaphore() -> &'static Semaphore {
    PDFIUM_SEMAPHORE.get_or_init(|| Semaphore::new(1))
}

/// Extract the first page of a PDF as a JPEG image (async version with concurrency control).
///
/// This is the preferred entry point for server use. It acquires a semaphore
/// to ensure only one PDF is rendered at a time, preventing pdfium segfaults.
#[cfg(feature = "media-metadata")]
pub async fn extract_cover_async(path: &Path) -> Result<Vec<u8>, PdfMetadataError> {
    let path = path.to_path_buf();
    let _permit = pdfium_semaphore().acquire().await.map_err(|_| {
        PdfMetadataError::RenderFailed("Failed to acquire pdfium semaphore".to_string())
    })?;

    // Run the blocking pdfium operation in a separate thread
    tokio::task::spawn_blocking(move || extract_cover_sync(&path))
        .await
        .map_err(|e| PdfMetadataError::RenderFailed(format!("Task join error: {}", e)))?
}

/// Extract the first page of a PDF as a JPEG image (sync version).
///
/// Uses pdfium-render to render the first page at a reasonable resolution
/// (max width 1200px, preserving aspect ratio).
///
/// **Warning**: This function is NOT thread-safe when called concurrently.
/// For server use, prefer `extract_cover_async` which handles concurrency.
#[cfg(feature = "media-metadata")]
pub fn extract_cover(path: &Path) -> Result<Vec<u8>, PdfMetadataError> {
    extract_cover_sync(path)
}

/// Internal sync implementation of cover extraction.
#[cfg(feature = "media-metadata")]
fn extract_cover_sync(path: &Path) -> Result<Vec<u8>, PdfMetadataError> {
    use image::codecs::jpeg::JpegEncoder;
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

    // Convert to image and encode as JPEG (quality 85 for good size/quality balance)
    let image = bitmap.as_image();
    let mut jpg_bytes = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut jpg_bytes, 85);
    image
        .write_with_encoder(encoder)
        .map_err(|e| PdfMetadataError::EncodeFailed(format!("JPEG encode failed: {}", e)))?;

    Ok(jpg_bytes)
}

/// Get the platform-specific pdfium library filename.
#[cfg(feature = "media-metadata")]
fn pdfium_lib_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "libpdfium.dylib"
    } else if cfg!(target_os = "windows") {
        "pdfium.dll"
    } else {
        "libpdfium.so"
    }
}

/// Get candidate paths for the pdfium library, in order of preference.
///
/// Search order:
/// 1. PDFIUM_DYNAMIC_LIB_PATH environment variable
/// 2. Next to the executable (for bundled releases)
/// 3. lib/ subdirectory next to executable
/// 4. macOS app bundle Frameworks directory
#[cfg(feature = "media-metadata")]
fn pdfium_candidate_paths() -> Vec<PathBuf> {
    let lib_name = pdfium_lib_name();
    let mut candidates = Vec::new();

    // 1. Environment variable (explicit override)
    if let Ok(lib_path) = std::env::var("PDFIUM_DYNAMIC_LIB_PATH") {
        candidates.push(PathBuf::from(&lib_path).join(lib_name));
    }

    // Get executable directory for relative paths
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(exe_dir) = exe_path.parent()
    {
        // 2. Next to executable (e.g., /usr/local/bin/libpdfium.dylib)
        candidates.push(exe_dir.join(lib_name));

        // 3. lib/ subdirectory (e.g., /usr/local/bin/lib/libpdfium.dylib)
        candidates.push(exe_dir.join("lib").join(lib_name));

        // 4. macOS app bundle: Contents/Frameworks/
        // Executable is at Contents/MacOS/mbr, so go up to Contents/
        if cfg!(target_os = "macos")
            && let Some(contents_dir) = exe_dir.parent()
        {
            candidates.push(contents_dir.join("Frameworks").join(lib_name));
        }
    }

    candidates
}

/// Create a Pdfium instance by searching multiple locations.
///
/// Search order:
/// 1. PDFIUM_DYNAMIC_LIB_PATH environment variable
/// 2. Next to the executable (for bundled releases)
/// 3. lib/ subdirectory next to executable
/// 4. macOS app bundle Frameworks directory
/// 5. System library search (fallback)
#[cfg(feature = "media-metadata")]
fn create_pdfium_instance() -> Result<pdfium_render::prelude::Pdfium, PdfMetadataError> {
    use pdfium_render::prelude::Pdfium;

    // Try each candidate path in order
    for candidate in pdfium_candidate_paths() {
        if candidate.exists() {
            tracing::debug!("Attempting to load pdfium from: {:?}", candidate);
            match Pdfium::bind_to_library(&candidate) {
                Ok(bindings) => {
                    tracing::debug!("Successfully loaded pdfium from {:?}", candidate);
                    return Ok(Pdfium::new(bindings));
                }
                Err(e) => {
                    tracing::warn!("Failed to bind to pdfium at {:?}: {}", candidate, e);
                    // Continue trying other locations
                }
            }
        } else {
            tracing::trace!("Pdfium not found at {:?}", candidate);
        }
    }

    // Fall back to system library search
    tracing::debug!("Attempting to load pdfium from system library");
    match Pdfium::bind_to_system_library() {
        Ok(bindings) => {
            tracing::debug!("Successfully loaded pdfium from system library");
            Ok(Pdfium::new(bindings))
        }
        Err(e) => {
            tracing::warn!("Failed to load pdfium from system library: {}", e);
            Err(PdfMetadataError::RenderFailed(format!(
                "Pdfium library not found. Install pdfium or set PDFIUM_DYNAMIC_LIB_PATH. Error: {}",
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
/// Creates a file named `{path}.cover.jpg` next to the PDF file.
/// Returns the path to the created sidecar file on success.
///
/// # Examples
/// ```ignore
/// let sidecar = save_cover(Path::new("docs/report.pdf"))?;
/// assert_eq!(sidecar, PathBuf::from("docs/report.pdf.cover.jpg"));
/// ```
#[cfg(feature = "media-metadata")]
pub fn save_cover(path: &Path) -> Result<PathBuf, PdfMetadataError> {
    let jpg_bytes = extract_cover(path)?;

    let sidecar_path = PathBuf::from(format!("{}.cover.jpg", path.display()));

    std::fs::write(&sidecar_path, jpg_bytes)?;

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
/// and creates `.cover.jpg` sidecar files for each.
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
            parse_pdf_cover_request("docs/report.pdf.cover.jpg"),
            Some("docs/report.pdf")
        );
        assert_eq!(
            parse_pdf_cover_request("docs/REPORT.PDF.cover.jpg"),
            Some("docs/REPORT.PDF")
        );
        assert_eq!(parse_pdf_cover_request("a.pdf.cover.jpg"), Some("a.pdf"));
        // Test with spaces in path
        assert_eq!(
            parse_pdf_cover_request("some dir/my file.pdf.cover.jpg"),
            Some("some dir/my file.pdf")
        );
    }

    #[test]
    fn test_parse_pdf_cover_request_invalid() {
        assert_eq!(parse_pdf_cover_request("docs/report.pdf"), None);
        assert_eq!(parse_pdf_cover_request("docs/report.png"), None);
        assert_eq!(parse_pdf_cover_request("docs/report.txt.cover.jpg"), None);
        assert_eq!(parse_pdf_cover_request("docs/report.cover.jpg"), None);
        assert_eq!(parse_pdf_cover_request("docs/report.pdf.cover.png"), None);
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
                // Verify JPEG magic bytes (0xFF 0xD8)
                assert!(bytes.len() > 2, "JPEG should have at least 2 bytes");
                assert_eq!(&bytes[0..2], &[0xFF, 0xD8], "Should be valid JPEG");
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
                    test_dir.path().join("test.pdf.cover.jpg"),
                    "Sidecar path should be {{pdf}}.cover.jpg"
                );

                // Verify file exists
                assert!(sidecar_path.exists(), "Sidecar file should exist");

                // Verify it's a valid JPEG
                let bytes = std::fs::read(&sidecar_path).expect("read sidecar");
                assert!(bytes.len() > 2, "JPEG should have at least 2 bytes");
                assert_eq!(&bytes[0..2], &[0xFF, 0xD8], "Should be valid JPEG");
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
            test_dir.path().join("test.pdf.cover.jpg").exists(),
            "First sidecar should exist"
        );
        assert!(
            subdir.join("test2.pdf.cover.jpg").exists(),
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
            let full_path = format!("{}.cover.jpg", base);

            let result = parse_pdf_cover_request(&full_path);
            prop_assert!(result.is_some(), "Valid PDF cover request should return Some");
            prop_assert_eq!(result.unwrap(), base.as_str());
        }

        /// Paths not ending in .cover.jpg always return None
        #[test]
        fn prop_non_cover_suffix_returns_none(
            path in "[a-zA-Z0-9_/.\\-]{1,50}",
            suffix in prop_oneof![".png", ".jpg", ".pdf", ".cover.png", ""]
        ) {
            // Skip if it accidentally ends with .cover.jpg
            let full_path = format!("{}{}", path, suffix);
            if full_path.ends_with(".cover.jpg") {
                return Ok(());
            }
            let result = parse_pdf_cover_request(&full_path);
            prop_assert!(result.is_none(), "Non .cover.jpg paths should return None");
        }

        /// Paths with .cover.jpg but not .pdf before it return None
        #[test]
        fn prop_non_pdf_cover_returns_none(
            name in pdf_basename_strategy(),
            ext in prop_oneof!["txt", "doc", "jpg", "png", ""]
        ) {
            let path = if ext.is_empty() {
                format!("{}.cover.jpg", name)
            } else {
                format!("{}.{}.cover.jpg", name, ext)
            };

            let result = parse_pdf_cover_request(&path);
            prop_assert!(result.is_none(), "Non-PDF .cover.jpg should return None: {}", path);
        }

        /// The returned base path when stripped from input leaves only .cover.jpg
        #[test]
        fn prop_base_path_plus_suffix_equals_input(
            dir in proptest::option::of(path_component_strategy()),
            name in pdf_basename_strategy()
        ) {
            let base = match dir {
                Some(d) => format!("{}/{}.pdf", d, name),
                None => format!("{}.pdf", name),
            };
            let full_path = format!("{}.cover.jpg", base);

            if let Some(result_base) = parse_pdf_cover_request(&full_path) {
                let reconstructed = format!("{}.cover.jpg", result_base);
                prop_assert_eq!(reconstructed, full_path);
            }
        }

        /// PDF extension case variations all work
        #[test]
        fn prop_pdf_extension_case_insensitive(
            name in pdf_basename_strategy()
        ) {
            let variations = [
                format!("{}.pdf.cover.jpg", name),
                format!("{}.PDF.cover.jpg", name),
                format!("{}.Pdf.cover.jpg", name),
                format!("{}.pDf.cover.jpg", name),
            ];

            for path in &variations {
                let result = parse_pdf_cover_request(path);
                prop_assert!(result.is_some(), "Should parse: {}", path);
            }
        }
    }
}
