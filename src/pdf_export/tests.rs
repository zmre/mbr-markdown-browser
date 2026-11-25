use super::*;

fn config_with(edit: impl FnOnce(&mut PdfConfig)) -> PdfConfig {
    let mut config = PdfConfig::default();
    edit(&mut config);
    config
}

fn no_default() -> Paper {
    panic!("the platform default must not be asked for")
}

// ---- contract ------------------------------------------------------------

/// The literals the front end names. `export-ready.test.ts` reads these
/// constants from this file; this pins the spelling on the Rust side.
#[test]
fn contract_literals_are_the_documented_ones() {
    assert_eq!(EXPORT_FLAG_GLOBAL, "__MBR_EXPORT__");
    assert_eq!(EXPORT_READY_FUNCTION, "mbrExportReady");
    assert_eq!(IPC_EXPORT_READY_PREFIX, "mbr:export-ready:");
    assert_eq!(IPC_EXPORT_FAILED_PREFIX, "mbr:export-failed:");
    assert_eq!(IPC_EDITOR_STATE_PREFIX, "mbr:editor-state:");
    assert_eq!(EXPORT_PAGE_STYLE_ID, "mbr-export-page");
}

// ---- options ---------------------------------------------------------------

#[test]
fn defaults_ask_the_platform_for_paper() {
    let options = PdfOptions::resolve(&PdfConfig::default(), &PdfFlags::default(), || Paper::Legal);
    assert_eq!(options.paper, Paper::Legal);
    assert_eq!(options.orientation, Orientation::Portrait);
    assert_eq!(options.margin.as_css(), "0.6in 0.5in");
    assert!(!options.page_numbers);
    assert_eq!(options.links, LinkPolicy::Strip);
    assert_eq!(options.timeout, Duration::from_secs(30));
}

#[test]
fn configuration_beats_the_platform_default() {
    let config = config_with(|c| {
        c.paper = Some(Paper::A4);
        c.orientation = Orientation::Landscape;
        c.margin = "1cm".parse().unwrap();
        c.page_numbers = true;
        c.link_base = Some("https://notes.example.com".parse().unwrap());
        c.timeout_ms = 1234;
    });
    let options = PdfOptions::resolve(&config, &PdfFlags::default(), no_default);
    assert_eq!(options.paper, Paper::A4);
    assert_eq!(options.orientation, Orientation::Landscape);
    assert_eq!(options.margin.as_css(), "1cm");
    assert!(options.page_numbers);
    assert_eq!(
        options.links,
        LinkPolicy::Base("https://notes.example.com/".parse().unwrap())
    );
    assert_eq!(options.timeout, Duration::from_millis(1234));
}

#[test]
fn flags_beat_the_configuration() {
    let config = config_with(|c| {
        c.paper = Some(Paper::A4);
        c.margin = "1cm".parse().unwrap();
        c.link_base = Some("https://a.example.com/".parse().unwrap());
        c.timeout_ms = 1234;
    });
    let flags = PdfFlags {
        paper: Some(Paper::Letter),
        orientation: Some(Orientation::Landscape),
        margin: Some("0".parse().unwrap()),
        page_numbers: Some(true),
        link_base: Some("https://b.example.com/".parse().unwrap()),
        timeout_ms: Some(99),
        ready: Some(ReadySignal::Load),
    };
    let options = PdfOptions::resolve(&config, &flags, no_default);
    assert_eq!(options.paper, Paper::Letter);
    assert_eq!(options.orientation, Orientation::Landscape);
    assert_eq!(options.ready, ReadySignal::Load);
    assert_eq!(options.margin.as_css(), "0");
    assert!(options.page_numbers);
    assert_eq!(
        options.links,
        LinkPolicy::Base("https://b.example.com/".parse().unwrap())
    );
    assert_eq!(options.timeout, Duration::from_millis(99));
}

/// A flag that was not given leaves the configuration's value alone, even
/// when that value is "on".
#[test]
fn absent_flags_do_not_reset_the_configuration() {
    let config = config_with(|c| {
        c.orientation = Orientation::Landscape;
        c.page_numbers = true;
    });
    let options = PdfOptions::resolve(&config, &PdfFlags::default(), || Paper::A4);
    assert_eq!(options.orientation, Orientation::Landscape);
    assert!(options.page_numbers);
}

/// The command line overrides the configuration in both directions: off
/// beats on as well as on beating off.
#[test]
fn flags_turn_configured_options_back_off() {
    let config = config_with(|c| {
        c.orientation = Orientation::Landscape;
        c.page_numbers = true;
    });
    let flags = PdfFlags {
        orientation: Some(Orientation::Portrait),
        page_numbers: Some(false),
        ..PdfFlags::default()
    };
    let options = PdfOptions::resolve(&config, &flags, || Paper::A4);
    assert_eq!(options.orientation, Orientation::Portrait);
    assert!(!options.page_numbers);
}

#[test]
fn landscape_swaps_the_paper_sides() {
    let mut options = PdfOptions::resolve(&PdfConfig::default(), &PdfFlags::default(), || {
        Paper::Letter
    });
    assert_eq!(options.page_size_in(), (8.5, 11.0));
    options.orientation = Orientation::Landscape;
    assert_eq!(options.page_size_in(), (11.0, 8.5));
}

#[test]
fn export_flag_is_the_contract_json() {
    let options = PdfOptions::resolve(&PdfConfig::default(), &PdfFlags::default(), || Paper::A4);
    let flag: serde_json::Value = serde_json::from_str(&options.export_flag_json()).unwrap();
    assert_eq!(
        flag,
        serde_json::json!({
            "links": "strip",
            "paperWidthIn": 8.2677,
            "paperHeightIn": 11.6929,
            "margin": "0.6in 0.5in",
            "ready": "page",
        })
    );

    let flags = PdfFlags {
        orientation: Some(Orientation::Landscape),
        link_base: Some("https://notes.example.com/docs".parse().unwrap()),
        ..PdfFlags::default()
    };
    let options = PdfOptions::resolve(&PdfConfig::default(), &flags, || Paper::Letter);
    let flag: serde_json::Value = serde_json::from_str(&options.export_flag_json()).unwrap();
    assert_eq!(
        flag["links"],
        serde_json::json!({"base": "https://notes.example.com/docs/"})
    );
    assert_eq!(flag["paperWidthIn"], 11.0);
    assert_eq!(flag["paperHeightIn"], 8.5);
}

#[test]
fn init_script_assigns_the_global() {
    let options = PdfOptions::resolve(&PdfConfig::default(), &PdfFlags::default(), || {
        Paper::Letter
    });
    let script = options.init_script();
    let prefix = format!("window.{EXPORT_FLAG_GLOBAL} = ");
    let json = script
        .strip_prefix(&prefix)
        .and_then(|rest| rest.strip_suffix(';'))
        .unwrap_or_else(|| panic!("unexpected script: {script}"));
    assert!(serde_json::from_str::<serde_json::Value>(json).is_ok());
    assert!(
        !script.contains("</script"),
        "never closes a script element"
    );
}

/// `ready = "load"`: the flag says so (the page's scripts stay out), and the
/// init script itself posts one ready message on `load` plus the fonts.
#[test]
fn ready_on_load_posts_from_the_init_script() {
    let config = config_with(|c| {
        c.ready = ReadySignal::Load;
        c.paper = Some(Paper::A4);
    });
    let options = PdfOptions::resolve(&config, &PdfFlags::default(), no_default);
    assert_eq!(options.ready, ReadySignal::Load);
    let flag: serde_json::Value = serde_json::from_str(&options.export_flag_json()).unwrap();
    assert_eq!(flag["ready"], "load");

    let script = options.init_script();
    let (assignment, signal) = script.split_once('\n').expect("two parts");
    assert!(assignment.starts_with(&format!("window.{EXPORT_FLAG_GLOBAL} = ")));
    assert!(signal.contains("addEventListener('load'"), "{signal}");
    assert!(signal.contains("document.fonts"), "{signal}");
    assert!(
        signal.contains(r#"ipc.postMessage("mbr:export-ready:{\"warnings\":[]}")"#),
        "{signal}"
    );
    assert!(!signal.contains("requestAnimationFrame"));
    assert!(!script.contains("</script"));

    // The flag beats the file, in either direction.
    let flags = PdfFlags {
        ready: Some(ReadySignal::Page),
        ..PdfFlags::default()
    };
    let options = PdfOptions::resolve(&config, &flags, no_default);
    assert_eq!(options.ready, ReadySignal::Page);
    assert!(!options.init_script().contains("addEventListener"));
}

// ---- a page's own @page size -------------------------------------------------

fn page(width_in: f64, height_in: f64) -> Option<PageSize> {
    Some(PageSize {
        width_in,
        height_in,
    })
}

#[test]
fn no_page_size_keeps_the_options() {
    let options = PdfOptions::resolve(&PdfConfig::default(), &PdfFlags::default(), || {
        Paper::Letter
    });
    assert_eq!(options.with_page_size(None), options);
}

/// The declared sheet replaces the configured paper and orientation, and
/// prints the same size it was declared as.
#[test]
fn a_declared_page_size_wins_over_paper_and_orientation() {
    let mut options = PdfOptions::resolve(&PdfConfig::default(), &PdfFlags::default(), || {
        Paper::Letter
    });
    options.orientation = Orientation::Landscape;
    let round = |(w, h): (f64, f64)| ((w * 100.0).round() / 100.0, (h * 100.0).round() / 100.0);

    // A5 portrait: a custom sheet, as written.
    let a5 = options.with_page_size(page(5.8268, 8.2677));
    assert_eq!(
        a5.paper,
        Paper::Custom {
            width_in: 5.8268,
            height_in: 8.2677
        }
    );
    assert_eq!(a5.orientation, Orientation::Portrait);
    assert_eq!(round(a5.page_size_in()), (5.83, 8.27));
    // Everything else is untouched.
    assert_eq!(a5.margin, options.margin);
    assert_eq!(a5.links, options.links);

    // A4 landscape: named A4, turned.
    let a4 = options.with_page_size(page(11.6929, 8.2677));
    assert_eq!(a4.paper, Paper::A4);
    assert_eq!(a4.orientation, Orientation::Landscape);
    assert_eq!(round(a4.page_size_in()), (11.69, 8.27));
    assert_eq!(a4.sheet().orientation, Orientation::Landscape);

    // Letter portrait, over a landscape configuration.
    let letter = options.with_page_size(page(8.5, 11.0));
    assert_eq!(letter.paper, Paper::Letter);
    assert_eq!(letter.orientation, Orientation::Portrait);
    assert_eq!(letter.page_size_in(), (8.5, 11.0));
}
// ---- IPC parsing -----------------------------------------------------------

#[test]
fn ready_message_carries_its_warnings() {
    assert_eq!(
        parse_export_message(r#"mbr:export-ready:{"warnings":["image a.png did not load"]}"#),
        Some(ExportMessage::Ready {
            warnings: vec!["image a.png did not load".to_owned()],
            page: None,
        })
    );
    assert_eq!(
        parse_export_message(r#"mbr:export-ready:{"warnings":[]}"#),
        Some(ExportMessage::Ready {
            warnings: vec![],
            page: None
        })
    );
}

/// A page whose own `@page { size }` declares the sheet says so; a size mbr
/// cannot print on (or a malformed one) is ignored with a warning.
#[test]
fn ready_message_carries_the_page_size() {
    assert_eq!(
        parse_export_message(
            r#"mbr:export-ready:{"warnings":[],"page":{"widthIn":5.8268,"heightIn":8.2677}}"#
        ),
        Some(ExportMessage::Ready {
            warnings: vec![],
            page: Some(PageSize {
                width_in: 5.8268,
                height_in: 8.2677
            }),
        })
    );
    for bad in [
        r#"{"widthIn":0.5,"heightIn":8}"#,
        r#"{"widthIn":5,"heightIn":500}"#,
        r#"{"widthIn":"5","heightIn":8}"#,
        r#"{"widthIn":5}"#,
        r#""a4""#,
        "null",
    ] {
        match parse_export_message(&format!(
            r#"mbr:export-ready:{{"warnings":[],"page":{bad}}}"#
        )) {
            Some(ExportMessage::Ready { warnings, page }) => {
                assert_eq!(page, None, "{bad}");
                assert_eq!(warnings.len(), 1, "{bad}: {warnings:?}");
                assert!(warnings[0].contains("@page size"), "{warnings:?}");
            }
            other => panic!("{bad} gave {other:?}"),
        }
    }
}

#[test]
fn failed_message_carries_its_reason() {
    assert_eq!(
        parse_export_message("mbr:export-failed:TypeError: x is undefined"),
        Some(ExportMessage::Failed {
            message: "TypeError: x is undefined".to_owned()
        })
    );
    assert_eq!(
        parse_export_message("mbr:export-failed:"),
        Some(ExportMessage::Failed {
            message: "no reason given".to_owned()
        })
    );
}

#[test]
fn other_messages_are_not_export_messages() {
    for body in [
        "",
        "mbr:open-external:https://example.com/",
        "mbr:export-readyX{}",
        "MBR:EXPORT-READY:{}",
        " mbr:export-ready:{}",
        "mbr:editor-state:open",
    ] {
        assert_eq!(parse_export_message(body), None, "{body:?}");
    }
}

/// The payload is page-controlled: malformed JSON still means ready (with a
/// warning), non-string warnings are dropped, sizes are capped and control
/// characters (terminal escapes) never survive.
#[test]
fn hostile_ready_payloads_are_contained() {
    let malformed = |body: &str| match parse_export_message(body) {
        Some(ExportMessage::Ready { warnings, .. }) => warnings,
        other => panic!("{body:?} gave {other:?}"),
    };
    assert_eq!(malformed("mbr:export-ready:not json").len(), 1);
    assert_eq!(malformed("mbr:export-ready:[1,2]").len(), 1);
    assert_eq!(malformed(r#"mbr:export-ready:{"warnings":"x"}"#).len(), 1);
    assert_eq!(
        malformed(r#"mbr:export-ready:{"warnings":[1,null,{"a":1},"ok"]}"#),
        vec!["ok".to_owned()]
    );

    let many = format!(
        "mbr:export-ready:{{\"warnings\":[{}]}}",
        vec!["\"w\""; 1000].join(",")
    );
    assert_eq!(malformed(&many).len(), MAX_WARNINGS);

    let long = format!(
        "mbr:export-ready:{{\"warnings\":[\"{}\"]}}",
        "x".repeat(5000)
    );
    let warning = &malformed(&long)[0];
    assert_eq!(warning.chars().count(), MAX_WARNING_CHARS);
    assert!(warning.ends_with('…'));

    let escapes = "mbr:export-ready:{\"warnings\":[\"\\u001b[31mred\\u0007\\nnext\"]}";
    assert_eq!(malformed(escapes), vec!["[31mred next".to_owned()]);

    let huge = format!("mbr:export-ready:{}", " ".repeat(MAX_IPC_BYTES));
    assert_eq!(parse_export_message(&huge), None);
}

#[test]
fn hostile_failure_messages_are_contained() {
    match parse_export_message(&format!("mbr:export-failed:{}", "é".repeat(5000))) {
        Some(ExportMessage::Failed { message }) => {
            assert_eq!(message.chars().count(), MAX_FAILURE_CHARS);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        parse_export_message("mbr:export-failed:\u{1b}]0;title\u{7}boom\r\n"),
        Some(ExportMessage::Failed {
            message: "]0;title boom".to_owned()
        })
    );
}

#[test]
fn editor_state_is_parsed_exactly() {
    assert_eq!(
        parse_editor_state("mbr:editor-state:open"),
        Some(EditorState::Open)
    );
    assert_eq!(
        parse_editor_state("mbr:editor-state:closed"),
        Some(EditorState::Closed)
    );
    for body in [
        "mbr:editor-state:",
        "mbr:editor-state:Open",
        "mbr:editor-state:open ",
        "mbr:editor-state:opened",
        "mbr:export-ready:{}",
        "editor-state:open",
    ] {
        assert_eq!(parse_editor_state(body), None, "{body:?}");
    }
}

// ---- the job ---------------------------------------------------------------

fn ready(warnings: &[&str]) -> JobEvent {
    JobEvent::Message(ExportMessage::Ready {
        warnings: warnings.iter().map(|w| (*w).to_owned()).collect(),
        page: None,
    })
}

#[test]
fn job_keeps_the_page_size_from_the_ready_message() {
    let mut job = PdfJob::new(Duration::from_secs(30));
    assert_eq!(job.page_size(), None);
    let page = PageSize {
        width_in: 5.8268,
        height_in: 8.2677,
    };
    let step = job.on_event(JobEvent::Message(ExportMessage::Ready {
        warnings: vec![],
        page: Some(page),
    }));
    assert_eq!(step, JobStep::Print);
    assert_eq!(job.page_size(), Some(page));
}

#[test]
fn job_without_a_ready_message_has_no_page_size() {
    let mut job = PdfJob::new(Duration::from_secs(30));
    assert_eq!(job.on_event(JobEvent::TimedOut), JobStep::Print);
    assert_eq!(job.page_size(), None);
}

#[test]
fn job_prints_once_ready_then_finishes() {
    let mut job = PdfJob::new(Duration::from_secs(30));
    assert_eq!(job.on_event(ready(&["slow image"])), JobStep::Print);
    assert_eq!(job.on_event(JobEvent::Printed(Ok(()))), JobStep::Finish);
    assert_eq!(job.warnings(), ["slow image"]);
}

#[test]
fn job_exports_anyway_on_timeout_with_a_warning() {
    let mut job = PdfJob::new(Duration::from_millis(1500));
    assert_eq!(job.on_event(JobEvent::TimedOut), JobStep::Print);
    assert_eq!(job.warnings().len(), 1);
    assert!(job.warnings()[0].contains("1.5 s"), "{:?}", job.warnings());
    // A late ready message changes nothing.
    assert_eq!(job.on_event(ready(&["late"])), JobStep::Wait);
    assert_eq!(job.on_event(JobEvent::Printed(Ok(()))), JobStep::Finish);
    assert_eq!(job.warnings().len(), 1);
}

#[test]
fn job_fails_when_the_page_or_the_print_does() {
    let mut job = PdfJob::new(Duration::from_secs(1));
    assert_eq!(
        job.on_event(JobEvent::Message(ExportMessage::Failed {
            message: "boom".to_owned()
        })),
        JobStep::Fail("boom".to_owned())
    );
    assert_eq!(job.on_event(JobEvent::TimedOut), JobStep::Wait);

    let mut job = PdfJob::new(Duration::from_secs(1));
    assert_eq!(job.on_event(ready(&[])), JobStep::Print);
    assert_eq!(
        job.on_event(JobEvent::Printed(Err("no".to_owned()))),
        JobStep::Fail("no".to_owned())
    );
}

/// Every step happens once: repeated or out-of-order events are ignored.
#[test]
fn job_ignores_repeated_and_out_of_order_events() {
    let mut job = PdfJob::new(Duration::from_secs(1));
    assert_eq!(job.on_event(JobEvent::Printed(Ok(()))), JobStep::Wait);
    assert_eq!(job.on_event(ready(&[])), JobStep::Print);
    assert_eq!(job.on_event(ready(&[])), JobStep::Wait);
    assert_eq!(job.on_event(JobEvent::TimedOut), JobStep::Wait);
    assert_eq!(job.on_event(JobEvent::Printed(Ok(()))), JobStep::Finish);
    assert_eq!(job.on_event(JobEvent::Printed(Ok(()))), JobStep::Wait);
    assert_eq!(job.on_event(ready(&[])), JobStep::Wait);
}

// ---- output ----------------------------------------------------------------

fn write_pdf(path: &Path) {
    std::fs::write(path, b"%PDF-1.7\n%stub\n").unwrap();
}

#[test]
fn output_is_verified_then_renamed_into_place() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("note.pdf");
    let pending = PendingOutput::prepare(OutputTarget::File(target.clone())).unwrap();
    let temp = pending.temp_path().to_path_buf();
    assert_eq!(temp.parent(), Some(dir.path()), "beside the target");
    assert!(!temp.exists(), "left for the print system to create");
    write_pdf(&temp);

    let bytes = pending.finish(&mut Vec::new()).unwrap();
    assert_eq!(bytes, 15);
    assert!(target.exists());
    assert!(!temp.exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn output_replaces_an_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("note.pdf");
    std::fs::write(&target, b"old").unwrap();
    let pending = PendingOutput::prepare(OutputTarget::File(target.clone())).unwrap();
    write_pdf(pending.temp_path());
    pending.finish(&mut Vec::new()).unwrap();
    assert!(std::fs::read(&target).unwrap().starts_with(b"%PDF-"));
}

#[test]
fn output_to_stdout_copies_the_bytes() {
    let pending = PendingOutput::prepare(OutputTarget::Stdout).unwrap();
    write_pdf(pending.temp_path());
    let temp = pending.temp_path().to_path_buf();
    let mut out = Vec::new();
    pending.finish(&mut out).unwrap();
    assert_eq!(out, b"%PDF-1.7\n%stub\n");
    assert!(!temp.exists(), "the temporary copy is removed");
}

#[test]
fn a_missing_or_wrong_file_is_refused_and_nothing_is_left() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("note.pdf");

    // The print system "succeeded" but wrote nothing (macOS, missing folder).
    let pending = PendingOutput::prepare(OutputTarget::File(target.clone())).unwrap();
    assert!(matches!(
        pending.finish(&mut Vec::new()),
        Err(PdfError::NotWritten { .. })
    ));

    let pending = PendingOutput::prepare(OutputTarget::File(target.clone())).unwrap();
    std::fs::write(pending.temp_path(), b"").unwrap();
    assert!(matches!(
        pending.finish(&mut Vec::new()),
        Err(PdfError::Empty { .. })
    ));

    let pending = PendingOutput::prepare(OutputTarget::File(target.clone())).unwrap();
    std::fs::write(pending.temp_path(), b"<html>").unwrap();
    assert!(matches!(
        pending.finish(&mut Vec::new()),
        Err(PdfError::NotAPdf { .. })
    ));

    assert!(!target.exists());
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "no temporary file left behind"
    );
}

#[test]
fn dropping_an_abandoned_output_removes_its_file() {
    let dir = tempfile::tempdir().unwrap();
    let pending = PendingOutput::prepare(OutputTarget::File(dir.path().join("a.pdf"))).unwrap();
    write_pdf(pending.temp_path());
    drop(pending);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn a_missing_folder_is_a_clear_error() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope");
    match PendingOutput::prepare(OutputTarget::File(missing.join("a.pdf"))) {
        Err(PdfError::OutputDirMissing { dir }) => assert_eq!(dir, missing),
        other => panic!("{other:?}"),
    }
    let file = dir.path().join("file");
    std::fs::write(&file, b"x").unwrap();
    assert!(matches!(
        PendingOutput::prepare(OutputTarget::File(file.join("a.pdf"))),
        Err(PdfError::OutputDirNotDirectory { .. })
    ));
}

#[cfg(unix)]
#[test]
fn an_unwritable_folder_is_a_clear_error() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let locked = dir.path().join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
    let result = PendingOutput::prepare(OutputTarget::File(locked.join("a.pdf")));
    // Root can write anywhere; the check is only meaningful for a user.
    let writable_anyway = std::fs::File::create(locked.join("probe")).is_ok();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
    if !writable_anyway {
        assert!(
            matches!(result, Err(PdfError::OutputDirNotWritable { .. })),
            "{result:?}"
        );
    }
}

#[test]
fn concurrent_temp_names_never_collide() {
    let dir = tempfile::tempdir().unwrap();
    let target = OutputTarget::File(dir.path().join("same.pdf"));
    let a = PendingOutput::prepare(target.clone()).unwrap();
    let b = PendingOutput::prepare(target).unwrap();
    assert_ne!(a.temp_path(), b.temp_path());
}

#[test]
fn verify_pdf_reports_the_size() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.pdf");
    write_pdf(&path);
    assert_eq!(verify_pdf(&path).unwrap(), 15);
    std::fs::write(&path, b"%PDF").unwrap();
    assert!(matches!(verify_pdf(&path), Err(PdfError::NotAPdf { .. })));
}

// ---- navigation ------------------------------------------------------------

fn export_origins(page: &str) -> ExportOrigins {
    ExportOrigins::new(page, NavigationAllowlist::default()).expect("a usable export URL")
}

#[test]
fn the_export_view_stays_on_its_own_origin() {
    let origins = export_origins("http://127.0.0.1:5555/note/");
    for allowed in [
        "http://127.0.0.1:5555/note/",
        "http://127.0.0.1:5555/other/page/?x=1#y",
        "HTTP://127.0.0.1:5555/redirected/",
        "blob:http://127.0.0.1:5555/0c1d-uuid",
        "about:blank",
        "about:srcdoc",
    ] {
        assert!(origins.allows(allowed), "{allowed}");
    }
    for refused in [
        // A data: document escapes the host's CSP and could load
        // `<img src=https://evil/?secret>` from its own (opaque) origin.
        "data:text/html,x",
        "data:text/html,<img src=https://evil.example/?k>",
        // Another origin, or the same host on another port or scheme.
        "https://evil.example/?secret",
        "https://www.youtube.com/embed/x",
        "http://127.0.0.1:5556/note/",
        "http://127.0.0.1/note/",
        "https://127.0.0.1:5555/note/",
        "http://localhost:5555/note/",
        "ws://127.0.0.1:5555/",
        // Userinfo that looks like the page's host.
        "http://127.0.0.1:5555@evil.example/",
        "http://127.0.0.1:5555:x@evil.example/",
        // A blob another origin made.
        "blob:https://evil.example/0c1d",
        "blob:null/0c1d",
        // Other about: pages, and hand-offs to other applications.
        "about:config",
        "mailto:a@example.com",
        "file:///etc/passwd",
        "zoommtg://x",
        "javascript:alert(1)",
        "",
        "not a url",
    ] {
        assert!(!origins.allows(refused), "{refused}");
    }
}

#[test]
fn the_export_origin_normalizes_default_ports_and_case() {
    let origins = export_origins("HTTP://LocalHost/note/");
    assert!(origins.allows("http://localhost:80/x/"));
    assert!(origins.allows("http://LOCALHOST/x/"));
    assert!(!origins.allows("http://localhost:8080/x/"));
    let origins = export_origins("https://notes.example:443/a/");
    assert!(origins.allows("https://notes.example/b/"));
    assert!(!origins.allows("http://notes.example/b/"));
}

#[test]
fn an_allowlisted_origin_is_allowed_and_nothing_else_is() {
    let allowlist = NavigationAllowlist::new()
        .allow_origin("https://www.youtube.com/embed/ignored-path")
        .unwrap()
        .allow_origin("http://assets.example:8080")
        .unwrap();
    assert_eq!(
        allowlist.origins().collect::<Vec<_>>(),
        ["https://www.youtube.com", "http://assets.example:8080"]
    );
    let origins = ExportOrigins::new("http://127.0.0.1:5555/note/", allowlist).unwrap();
    assert!(origins.allows("https://www.youtube.com/embed/x"));
    assert!(origins.allows("http://assets.example:8080/a.png"));
    assert!(origins.allows("http://127.0.0.1:5555/note/"));
    assert!(!origins.allows("http://www.youtube.com/embed/x"));
    assert!(!origins.allows("https://youtube.com/embed/x"));
    assert!(!origins.allows("http://assets.example/a.png"));
    assert!(!origins.allows("https://evil.example/"));
}

#[test]
fn only_http_origins_can_be_allowed_or_exported() {
    for bad in [
        "",
        "not a url",
        "file:///tmp/x.html",
        "data:text/html,x",
        "about:blank",
        "javascript:alert(1)",
        "mbr://localhost/",
    ] {
        match NavigationAllowlist::new().allow_origin(bad) {
            Err(e @ PdfError::ExportUrl { .. }) => assert!(e.is_refusal()),
            other => panic!("{bad}: {other:?}"),
        }
        assert!(
            matches!(
                ExportOrigins::new(bad, NavigationAllowlist::default()),
                Err(PdfError::ExportUrl { .. })
            ),
            "{bad}"
        );
    }
}

// ---- Linux preflight (pure, so tested everywhere) --------------------------

#[test]
fn linux_preflight_passes_when_gtk_starts() {
    assert!(linux_preflight_decision(None, None, Ok(())).is_ok());
    assert!(linux_preflight_decision(Some(":0"), None, Ok(())).is_ok());
}

#[test]
fn linux_preflight_without_a_display_says_so() {
    for (display, wayland) in [(None, None), (Some(""), Some("  "))] {
        match linux_preflight_decision(display, wayland, Err("cannot open display".into())) {
            Err(e @ PdfError::NoDisplay(_)) => {
                assert!(e.is_refusal());
                let message = e.to_string();
                assert!(message.contains("DISPLAY"), "{message}");
                assert!(message.contains("xvfb-run"), "{message}");
            }
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn linux_preflight_with_a_broken_display_names_it() {
    match linux_preflight_decision(Some(":99"), Some("wayland-0"), Err("refused".into())) {
        Err(PdfError::NoDisplay(message)) => {
            assert!(message.contains(":99, wayland-0"), "{message}");
            assert!(message.contains("refused"), "{message}");
        }
        other => panic!("{other:?}"),
    }
}

/// Exit code 2 ("refused") is everything detected before the export window
/// exists; 1 is a failure once it ran.
#[test]
fn refusals_are_what_is_detected_before_the_export_starts() {
    let dir = PathBuf::from("/nowhere");
    let refused = [
        PdfError::Unsupported { platform: "x" },
        PdfError::NoDisplay("no display".to_owned()),
        PdfError::OutputDirMissing { dir: dir.clone() },
        PdfError::OutputDirNotDirectory { dir: dir.clone() },
        PdfError::OutputDirNotWritable {
            dir: dir.clone(),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        },
        PdfError::Server("bind".to_owned()),
    ];
    for error in &refused {
        assert!(error.is_refusal(), "{error:?}");
    }
    let failed = [
        PdfError::PageFailed {
            message: String::new(),
        },
        PdfError::PrintFailed {
            message: String::new(),
        },
        PdfError::NotWritten { path: dir.clone() },
        PdfError::Empty { path: dir.clone() },
        PdfError::NotAPdf { path: dir.clone() },
        PdfError::Finalize {
            path: dir,
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        },
        PdfError::Stdout(std::io::Error::from(std::io::ErrorKind::BrokenPipe)),
    ];
    for error in &failed {
        assert!(!error.is_refusal(), "{error:?}");
    }
}

/// A missing output folder is found by `PendingOutput::prepare`, before
/// anything renders, and is therefore a refusal.
#[test]
fn a_missing_output_folder_is_a_refusal() {
    let dir = tempfile::tempdir().unwrap();
    let target = OutputTarget::File(dir.path().join("missing").join("x.pdf"));
    match PendingOutput::prepare(target) {
        Err(e @ PdfError::OutputDirMissing { .. }) => assert!(e.is_refusal()),
        other => panic!("{other:?}"),
    }
}

// ---- sheet (AppKit, GTK) ----------------------------------------------------

fn options_for(paper: Paper, orientation: Orientation) -> PdfOptions {
    let mut options = PdfOptions::resolve(&PdfConfig::default(), &PdfFlags::default(), || paper);
    options.orientation = orientation;
    options
}

#[test]
fn a_portrait_page_is_an_unturned_sheet() {
    let sheet = options_for(Paper::Letter, Orientation::Portrait).sheet();
    assert_eq!(
        sheet,
        Sheet {
            width_in: 8.5,
            height_in: 11.0,
            orientation: Orientation::Portrait
        }
    );
}

/// The print system turns the sheet itself, so the sides it is given stay
/// portrait: turning them here as well would rotate the page twice.
#[test]
fn a_landscape_page_is_a_portrait_sheet_turned_once() {
    let sheet = options_for(Paper::A4, Orientation::Landscape).sheet();
    let (a4_w, a4_h) = Paper::A4.dimensions_in();
    assert_eq!(
        sheet,
        Sheet {
            width_in: a4_w,
            height_in: a4_h,
            orientation: Orientation::Landscape
        }
    );
}

#[test]
fn a_custom_paper_written_wide_is_a_turned_sheet() {
    let wide = Paper::Custom {
        width_in: 10.0,
        height_in: 4.0,
    };
    let portrait = options_for(wide, Orientation::Portrait).sheet();
    assert_eq!(
        (portrait.width_in, portrait.height_in, portrait.orientation),
        (4.0, 10.0, Orientation::Landscape)
    );
    // Asked for landscape, the wide paper stands up again.
    let landscape = options_for(wide, Orientation::Landscape).sheet();
    assert_eq!(
        (
            landscape.width_in,
            landscape.height_in,
            landscape.orientation
        ),
        (4.0, 10.0, Orientation::Portrait)
    );
}

#[test]
fn a_sheet_always_has_portrait_sides_and_the_page_size() {
    let papers = [
        Paper::Letter,
        Paper::A4,
        Paper::Legal,
        Paper::Custom {
            width_in: 3.0,
            height_in: 3.0,
        },
        Paper::Custom {
            width_in: 12.0,
            height_in: 5.0,
        },
    ];
    for paper in papers {
        for orientation in [Orientation::Portrait, Orientation::Landscape] {
            let options = options_for(paper, orientation);
            let sheet = options.sheet();
            assert!(
                sheet.width_in <= sheet.height_in,
                "{paper:?} {orientation:?}"
            );
            let turned = match sheet.orientation {
                Orientation::Portrait => (sheet.width_in, sheet.height_in),
                Orientation::Landscape => (sheet.height_in, sheet.width_in),
            };
            assert_eq!(turned, options.page_size_in(), "{paper:?} {orientation:?}");
        }
    }
}

// ---- Linux paper names (pure, so tested everywhere) ------------------------

#[test]
fn gtk_paper_names_map_both_ways() {
    for paper in [Paper::Letter, Paper::A4, Paper::Legal] {
        let name = gtk_name_for_paper(paper).expect("a standard paper has a GTK name");
        assert_eq!(paper_for_gtk_name(name), Some(paper), "{name}");
    }
    assert_eq!(gtk_name_for_paper(Paper::A4), Some("iso_a4"));
    assert_eq!(gtk_name_for_paper(Paper::Letter), Some("na_letter"));
    assert_eq!(
        gtk_name_for_paper(Paper::Custom {
            width_in: 5.0,
            height_in: 7.0
        }),
        None
    );
    for unknown in ["iso_a5", "", "A4", "na_letter "] {
        assert_eq!(paper_for_gtk_name(unknown), None, "{unknown:?}");
    }
}

// ---- Windows locale buffer (pure, so tested everywhere) --------------------

fn utf16_buffer(text: &str) -> (Vec<u16>, i32) {
    let mut buffer: Vec<u16> = text.encode_utf16().collect();
    buffer.push(0);
    let returned = i32::try_from(buffer.len()).expect("small");
    buffer.resize(85, 0);
    (buffer, returned)
}

#[test]
fn windows_locale_name_is_read_without_its_nul() {
    let (buffer, returned) = utf16_buffer("en-US");
    assert_eq!(
        locale_from_utf16(&buffer, returned).as_deref(),
        Some("en-US")
    );
    let (buffer, returned) = utf16_buffer("fr-CA");
    let locale = locale_from_utf16(&buffer, returned).expect("a locale");
    assert_eq!(crate::pdf_config::paper_for_locale(&locale), Paper::Letter);
    let (buffer, returned) = utf16_buffer("de-DE");
    let locale = locale_from_utf16(&buffer, returned).expect("a locale");
    assert_eq!(crate::pdf_config::paper_for_locale(&locale), Paper::A4);
}

#[test]
fn a_failed_or_odd_windows_locale_call_is_no_locale() {
    let (buffer, _) = utf16_buffer("en-US");
    // 0 = failure; 1 = just the NUL; past the buffer; negative.
    for returned in [0, 1, 87, -1] {
        assert_eq!(locale_from_utf16(&buffer, returned), None, "{returned}");
    }
    // A count past the string's NUL still reads only the string.
    assert_eq!(locale_from_utf16(&buffer, 86).as_deref(), Some("en-US"));
    let (buffer, returned) = utf16_buffer("  ");
    assert_eq!(locale_from_utf16(&buffer, returned), None);
    // Unpaired surrogate: not a name.
    let mut bad = vec![0xD800_u16, 0];
    bad.resize(85, 0);
    assert_eq!(locale_from_utf16(&bad, 2), None);
}
