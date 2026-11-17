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
    };
    let options = PdfOptions::resolve(&config, &flags, no_default);
    assert_eq!(options.paper, Paper::Letter);
    assert_eq!(options.orientation, Orientation::Landscape);
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

// ---- IPC parsing -----------------------------------------------------------

#[test]
fn ready_message_carries_its_warnings() {
    assert_eq!(
        parse_export_message(r#"mbr:export-ready:{"warnings":["image a.png did not load"]}"#),
        Some(ExportMessage::Ready {
            warnings: vec!["image a.png did not load".to_owned()]
        })
    );
    assert_eq!(
        parse_export_message(r#"mbr:export-ready:{"warnings":[]}"#),
        Some(ExportMessage::Ready { warnings: vec![] })
    );
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
        Some(ExportMessage::Ready { warnings }) => warnings,
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
    })
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

#[test]
fn the_export_view_never_hands_off_to_another_application() {
    for allowed in [
        "http://127.0.0.1:5555/note/",
        "https://www.youtube.com/embed/x",
        "about:blank",
        "data:text/html,x",
        "blob:http://127.0.0.1/x",
    ] {
        assert!(export_navigation_allowed(allowed), "{allowed}");
    }
    for refused in [
        "mailto:a@example.com",
        "file:///etc/passwd",
        "zoommtg://x",
        "javascript:alert(1)",
        "",
    ] {
        assert!(!export_navigation_allowed(refused), "{refused}");
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
