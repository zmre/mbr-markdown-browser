//! `mbr --pdf <note>`: export one note to a PDF with no window, no Dock icon
//! and no dialog. See [`crate::pdf_export`] for how the export works.
//!
//! The note is served by an in-process server on an ephemeral loopback port
//! (never the configured host or port, and with editing off), read with the
//! same configuration layers as `mbr -s`. The exit code says what happened:
//! [`EXIT_WRITTEN`], [`EXIT_FAILED`] (cause and path on stderr) or
//! [`EXIT_REFUSED`] (nothing was started). Logs and warnings go to stderr, so
//! `--output -` can put the PDF on stdout.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use parking_lot::Mutex;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::run_return::EventLoopExtRunReturn;

use crate::cli::Args;
use crate::config::Config;
use crate::errors::PdfError;
use crate::pdf_export::{
    self, ExportEvent, ExportSession, ExportSink, ExportedPdf, OutputTarget, PdfOptions,
    PendingOutput,
};
use crate::server::{Server, ServerConfig};

/// The PDF was written.
pub const EXIT_WRITTEN: u8 = 0;
/// The export started and failed.
pub const EXIT_FAILED: u8 = 1;
/// Refused before starting: bad options, a missing note, an output folder
/// that is missing or not writable, no display, no backend (anything found
/// before the export window exists).
pub const EXIT_REFUSED: u8 = 2;

/// The `--output` value that means standard output.
const STDOUT_MARKER: &str = "-";

/// Where `mbr --pdf <note> [--output <output>]` writes, relative to `cwd`:
///
/// - no `--output`: `<cwd>/<note stem>.pdf`;
/// - `-`: standard output;
/// - an existing folder, or a path ending in a separator: `<folder>/<stem>.pdf`;
/// - anything else: that file.
///
/// A folder note is named after the folder (`docs/` → `docs.pdf`); the
/// repository root after nothing better than `index.pdf`.
pub fn pdf_output_path(note: &Path, output: Option<&Path>, cwd: &Path) -> OutputTarget {
    let file_name = format!("{}.pdf", note_stem(note));
    match output {
        None => OutputTarget::File(cwd.join(file_name)),
        Some(path) if path.as_os_str() == STDOUT_MARKER => OutputTarget::Stdout,
        Some(path) => {
            let resolved = cwd.join(path);
            let names_a_folder = path
                .as_os_str()
                .to_string_lossy()
                .ends_with(std::path::is_separator)
                || resolved.is_dir();
            OutputTarget::File(if names_a_folder {
                resolved.join(file_name)
            } else {
                resolved
            })
        }
    }
}

/// The name a note's PDF is given: its file stem, or a folder's name.
fn note_stem(note: &Path) -> String {
    let stem = if note.is_dir() {
        note.file_name()
    } else {
        note.file_stem()
    };
    stem.map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty() && s != "." && s != "..")
        .unwrap_or_else(|| "index".to_owned())
}

/// Runs `mbr --pdf`. `absolute_path` is the canonical note, `relative_path`
/// the note relative to `config.root_dir`; `config` was read like server
/// mode's, flags applied.
///
/// Blocks the calling thread in a tao event loop, so call it from the main
/// thread (as `main`'s `#[tokio::main]` body runs).
pub async fn run(
    args: &Args,
    absolute_path: &Path,
    relative_path: &Path,
    config: Config,
) -> ExitCode {
    match export(args, absolute_path, relative_path, config).await {
        Ok(exported) => {
            for warning in &exported.warnings {
                eprintln!("Warning: {warning}");
            }
            tracing::info!("Wrote {} ({} bytes)", exported.target, exported.bytes);
            ExitCode::from(EXIT_WRITTEN)
        }
        Err(error) => {
            eprintln!("Error: {}", report(&error));
            ExitCode::from(if error.is_refusal() {
                EXIT_REFUSED
            } else {
                EXIT_FAILED
            })
        }
    }
}

/// An error and its sources on one line each, like `main`'s report.
fn report(error: &PdfError) -> String {
    let mut message = error.to_string();
    let mut source = std::error::Error::source(error);
    while let Some(cause) = source {
        message.push_str("\n  caused by: ");
        message.push_str(&cause.to_string());
        source = cause.source();
    }
    message
}

async fn export(
    args: &Args,
    absolute_path: &Path,
    relative_path: &Path,
    mut config: Config,
) -> Result<ExportedPdf, PdfError> {
    // Before anything else exists: no backend, or no display, is a refusal.
    pdf_export::preflight()?;

    let cwd = std::env::current_dir().map_err(|source| PdfError::OutputDirNotWritable {
        dir: PathBuf::from("."),
        source,
    })?;
    let target = pdf_output_path(absolute_path, args.output.as_deref(), &cwd);
    let output = PendingOutput::prepare(target)?;

    // A one-shot reader: nothing may write to the repository.
    config.edit_enabled = false;
    let is_directory = absolute_path.is_dir();
    let url_path = crate::launch_url::resolve_launch_url_path(
        relative_path,
        is_directory,
        &config.markdown_extensions,
    );
    let flags = args.pdf_flags();
    let pdf_config = config.pdf.clone();

    let server = Arc::new(
        Server::init(ServerConfig::from(&config)).map_err(|e| PdfError::Server(e.to_string()))?,
    );
    // Port 0: the system picks a free one, so a running `mbr -s` (or another
    // export) can never be in the way.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| PdfError::Server(e.to_string()))?;
    let port = listener
        .local_addr()
        .map_err(|e| PdfError::Server(e.to_string()))?
        .port();
    let serving = Arc::clone(&server);
    tokio::spawn(async move {
        if let Err(e) = serving.serve_on(listener).await {
            tracing::error!("PDF export server stopped: {e}");
        }
    });

    // Wikilinks, backlinks and the site index need the scan; it is bounded by
    // the same timeout as the page, and a slow one only costs completeness.
    let scan_budget =
        std::time::Duration::from_millis(flags.timeout_ms.unwrap_or(pdf_config.timeout_ms));
    if tokio::time::timeout(scan_budget, server.repo().wait_for_scan())
        .await
        .is_err()
    {
        eprintln!("Warning: the repository scan did not finish in time; exporting without it");
    }

    let url = format!("http://127.0.0.1:{port}/{url_path}");
    tracing::info!("Exporting {url} to {}", output.target());

    // Resolved here, on the main thread: the macOS default paper is read from
    // AppKit.
    let options = PdfOptions::resolve(&pdf_config, &flags, pdf_export::default_paper);
    let result = run_export_loop(&url, options, output);
    drop(server);
    result
}

/// One export on a tao event loop of its own, run to completion.
fn run_export_loop(
    url: &str,
    options: PdfOptions,
    output: PendingOutput,
) -> Result<ExportedPdf, PdfError> {
    let mut event_loop = EventLoopBuilder::<ExportEvent>::with_user_event().build();
    pdf_export::configure_cli_event_loop(&mut event_loop);
    let proxy = Mutex::new(event_loop.create_proxy());
    let sink: ExportSink = Arc::new(move |event| {
        // Fails only once the loop has ended, when nothing wants the event.
        let _ = proxy.lock().send_event(event);
    });

    let mut start = Some((options, output));
    let mut session: Option<ExportSession> = None;
    let mut result: Option<Result<ExportedPdf, PdfError>> = None;
    // Not locked for the loop's lifetime: other threads may print meanwhile.
    let mut stdout = std::io::stdout();

    event_loop.run_return(|event, target, control_flow| {
        *control_flow = ControlFlow::Wait;
        let finished = match event {
            Event::NewEvents(StartCause::Init) => start.take().and_then(|(options, output)| {
                match ExportSession::start(target, url, options, output, Arc::clone(&sink)) {
                    Ok(started) => {
                        session = Some(started);
                        None
                    }
                    Err(e) => Some(Err(e)),
                }
            }),
            Event::UserEvent(event) => session
                .as_mut()
                .and_then(|session| session.handle(event, &mut stdout)),
            _ => None,
        };
        if let Some(finished) = finished {
            result = Some(finished);
            *control_flow = ControlFlow::Exit;
        }
    });
    drop(session);
    result.unwrap_or_else(|| {
        Err(PdfError::PrintFailed {
            message: "the export ended before it finished".to_owned(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_output_writes_beside_the_cwd_named_after_the_note() {
        let cwd = Path::new("/work");
        assert_eq!(
            pdf_output_path(Path::new("/notes/My Note.md"), None, cwd),
            OutputTarget::File(PathBuf::from("/work/My Note.pdf"))
        );
    }

    #[test]
    fn dash_is_stdout() {
        assert_eq!(
            pdf_output_path(Path::new("/n/a.md"), Some(Path::new("-")), Path::new("/w")),
            OutputTarget::Stdout
        );
    }

    #[test]
    fn a_file_output_is_used_as_given_relative_to_cwd() {
        let cwd = Path::new("/work");
        assert_eq!(
            pdf_output_path(Path::new("/n/a.md"), Some(Path::new("out/x.pdf")), cwd),
            OutputTarget::File(PathBuf::from("/work/out/x.pdf"))
        );
        assert_eq!(
            pdf_output_path(Path::new("/n/a.md"), Some(Path::new("/abs/y.pdf")), cwd),
            OutputTarget::File(PathBuf::from("/abs/y.pdf"))
        );
    }

    #[test]
    fn an_existing_folder_gets_the_note_name() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        std::fs::create_dir(&out).unwrap();
        assert_eq!(
            pdf_output_path(Path::new("/n/guide.md"), Some(&out), Path::new("/w")),
            OutputTarget::File(out.join("guide.pdf"))
        );
        // Relative to cwd, too.
        assert_eq!(
            pdf_output_path(Path::new("/n/guide.md"), Some(Path::new("out")), dir.path()),
            OutputTarget::File(out.join("guide.pdf"))
        );
    }

    /// A trailing separator means "a folder" even before it exists, so the
    /// export fails with "folder does not exist" instead of writing a file
    /// called `missing`.
    #[test]
    fn a_trailing_separator_names_a_folder() {
        let sep = std::path::MAIN_SEPARATOR;
        let output = format!("missing{sep}");
        assert_eq!(
            pdf_output_path(
                Path::new("/n/a.md"),
                Some(Path::new(&output)),
                Path::new("/w")
            ),
            OutputTarget::File(PathBuf::from("/w/missing").join("a.pdf"))
        );
    }

    #[test]
    fn a_folder_note_is_named_after_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let docs = dir.path().join("docs");
        std::fs::create_dir(&docs).unwrap();
        assert_eq!(
            pdf_output_path(&docs, None, Path::new("/w")),
            OutputTarget::File(PathBuf::from("/w/docs.pdf"))
        );
        assert_eq!(note_stem(Path::new("/")), "index");
    }
}
