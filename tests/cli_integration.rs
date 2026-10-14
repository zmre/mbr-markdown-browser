//! End-to-end tests that invoke the real `mbr` binary as a subprocess.
//!
//! Everything between `main()`'s first line and the library API — argument
//! parsing, the flag→`Config` overrides, and the process exit codes — is
//! unreachable from in-process tests. `tests/build_integration.rs` drives
//! `Builder` directly and never sees `--fail-on-broken-links`; `src/cli.rs`
//! asserts clap fills the struct. Only a subprocess covers the seam, so these
//! tests run `CARGO_BIN_EXE_mbr`, which Cargo points at the binary built from
//! the current source.
//!
//! Keep the fixtures tiny: every test here pays for a full static build.

#[allow(dead_code)] // Shared fixture; each test binary uses a different subset.
mod common;

use common::TestRepo;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

/// A markdown page whose only link points at a page that does not exist.
const BROKEN_LINK_PAGE: &str = "# Home\n\n[Broken](/missing/)\n";

/// Builds a `Command` for the binary under test with the ambient environment
/// neutralized. `MBR_*` variables are the config layer directly below CLI flags
/// and `RUST_LOG` outranks `-q`, so a developer's shell (or CI) could otherwise
/// change what these tests observe.
fn mbr_command() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mbr"));
    cmd.env_remove("RUST_LOG");
    for (key, _) in std::env::vars() {
        if key.starts_with("MBR_") {
            cmd.env_remove(key);
        }
    }
    cmd
}

/// Runs `mbr -b` against `repo`, writing the site into `output_dir`.
fn run_build(repo: &TestRepo, output_dir: &Path, extra_args: &[&str]) -> Output {
    mbr_command()
        .arg("-b")
        .arg("-q")
        .arg("--output")
        .arg(output_dir)
        .args(extra_args)
        .arg(repo.path())
        .output()
        .expect("failed to run the mbr binary")
}

/// A repo with exactly one page containing one broken internal link.
fn repo_with_broken_link() -> TestRepo {
    let repo = TestRepo::new();
    repo.create_markdown("index.md", BROKEN_LINK_PAGE);
    repo
}

/// A repo whose only link resolves.
fn repo_without_broken_links() -> TestRepo {
    let repo = TestRepo::new();
    repo.create_markdown("index.md", "# Home\n\n[Other](/other/)\n");
    repo.create_markdown("other.md", "# Other\n");
    repo
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

// ---- --fail-on-broken-links -------------------------------------------------

/// The CI gate. Without this test the flag could be deleted outright and every
/// check would stay green, because the docs tree CI builds it against has no
/// broken links.
#[test]
fn test_fail_on_broken_links_exits_nonzero_when_links_are_broken() {
    let repo = repo_with_broken_link();
    let out = TempDir::new().expect("temp output dir");
    let output = run_build(&repo, out.path(), &["--fail-on-broken-links"]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "expected exit code 1.\nstdout:\n{}\nstderr:\n{}",
        stdout_of(&output),
        stderr_of(&output)
    );
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("broken internal link"),
        "stderr should explain why the build failed, got:\n{stderr}"
    );
}

/// The same repo is a *successful* build without the flag: broken links are
/// reported but advisory. Asserting the count in the summary keeps this from
/// passing vacuously if link detection itself regresses.
#[test]
fn test_broken_links_without_flag_exits_zero() {
    let repo = repo_with_broken_link();
    let out = TempDir::new().expect("temp output dir");
    let output = run_build(&repo, out.path(), &[]);

    assert!(
        output.status.success(),
        "broken links alone must not fail the build.\nstderr:\n{}",
        stderr_of(&output)
    );
    let stdout = stdout_of(&output);
    assert!(
        stdout.contains("1 broken links"),
        "the build should still have detected the broken link, got:\n{stdout}"
    );
}

#[test]
fn test_fail_on_broken_links_exits_zero_on_clean_repo() {
    let repo = repo_without_broken_links();
    let out = TempDir::new().expect("temp output dir");
    let output = run_build(&repo, out.path(), &["--fail-on-broken-links"]);

    assert!(
        output.status.success(),
        "a clean repo must pass the CI gate.\nstdout:\n{}\nstderr:\n{}",
        stdout_of(&output),
        stderr_of(&output)
    );
    assert!(
        !stdout_of(&output).contains("broken links"),
        "a clean repo should not report broken links"
    );
}

/// Pins the precedence: `--skip-link-checks` wins. Validation never runs, so
/// `stats.broken_links` stays 0 and `--fail-on-broken-links` cannot fire — the
/// build exits 0 even though the repo *does* contain a broken link. This is
/// what `src/cli.rs`'s help text promises ("Has no effect with
/// --skip-link-checks"), and it is a foot-gun worth pinning: a CI job passing
/// both flags gets no gate at all.
#[test]
fn test_skip_link_checks_suppresses_fail_on_broken_links() {
    let repo = repo_with_broken_link();
    let out = TempDir::new().expect("temp output dir");
    let output = run_build(
        &repo,
        out.path(),
        &["--skip-link-checks", "--fail-on-broken-links"],
    );

    assert!(
        output.status.success(),
        "--skip-link-checks should suppress the gate.\nstdout:\n{}\nstderr:\n{}",
        stdout_of(&output),
        stderr_of(&output)
    );
    assert!(
        stdout_of(&output).contains("Validating links ... skipped"),
        "validation should have been skipped entirely"
    );
    assert!(
        !stderr_of(&output).contains("broken internal link"),
        "the gate must not fire when validation was skipped"
    );
    // The combination disarms the CI gate completely, so it must say so.
    // Silently exiting 0 here is how a repository full of broken links passes
    // a pipeline that looks like it is checking them.
    let stderr = stderr_of(&output);
    assert!(
        stderr.contains("--fail-on-broken-links has no effect"),
        "combining the two flags must warn that the gate is disarmed.\nstderr:\n{stderr}"
    );
}

/// The warning is specific to the disarmed combination: asking to fail on
/// broken links, without skipping the checks, must stay quiet.
#[test]
fn test_fail_on_broken_links_alone_does_not_warn_about_being_disabled() {
    let repo = repo_with_broken_link();
    let out = TempDir::new().expect("temp output dir");
    let output = run_build(&repo, out.path(), &["--fail-on-broken-links"]);

    let stderr = stderr_of(&output);
    assert!(
        !stderr.contains("has no effect"),
        "the gate was active here; there is nothing to warn about.\nstderr:\n{stderr}"
    );
}

// ---- flag → Config wiring ---------------------------------------------------

/// `cli::apply_overrides` is unit-tested, but nothing else proves `main()`
/// still calls it. This drives a flag all the way through to rendered output.
#[test]
fn test_title_prefix_and_suffix_reach_rendered_output() {
    let repo = TestRepo::new();
    repo.create_markdown("index.md", "# Home\n");
    let out = TempDir::new().expect("temp output dir");

    let output = run_build(
        &repo,
        out.path(),
        &["--title-prefix", "PFX: ", "--title-suffix", " | SFX"],
    );
    assert!(
        output.status.success(),
        "build failed:\n{}",
        stderr_of(&output)
    );

    let html = std::fs::read_to_string(out.path().join("index.html")).expect("read index.html");
    assert!(
        html.contains("<title>PFX: Home | SFX</title>"),
        "CLI title overrides did not reach the template, got:\n{}",
        html.lines().take(20).collect::<Vec<_>>().join("\n")
    );
}

/// `--host ::1` parses as an IP but cannot be stored in `Config`'s four-octet
/// host, so startup aborts. Exercised in build mode: it reaches the same
/// override code and is guaranteed to terminate (server mode would block).
#[test]
fn test_ipv6_host_is_rejected() {
    let repo = TestRepo::new();
    repo.create_markdown("index.md", "# Home\n");
    let out = TempDir::new().expect("temp output dir");

    let output = run_build(&repo, out.path(), &["--host", "::1"]);
    assert!(
        !output.status.success(),
        "an IPv6 host should abort startup"
    );
    assert!(
        stderr_of(&output).contains("InvalidHost"),
        "stderr should name the failure, got:\n{}",
        stderr_of(&output)
    );
}

/// `--template-folder` pointing at a file (not a directory) aborts startup.
#[test]
fn test_template_folder_that_is_a_file_is_rejected() {
    let repo = TestRepo::new();
    repo.create_markdown("index.md", "# Home\n");
    let not_a_dir = repo.path().join("index.md");
    let out = TempDir::new().expect("temp output dir");

    let output = run_build(
        &repo,
        out.path(),
        &["--template-folder", not_a_dir.to_str().expect("utf-8 path")],
    );
    assert!(
        !output.status.success(),
        "a file passed as --template-folder should abort startup"
    );
    assert!(
        stderr_of(&output).contains("TemplateFolderNotDirectory"),
        "stderr should name the failure, got:\n{}",
        stderr_of(&output)
    );
}

/// A symlink to `$HOME` is refused as a mount — it would publish everything
/// the user owns — and the build says why. `HOME` is a fake, set only on the
/// child process; a sibling link to a folder *inside* it is still a mount, and
/// the build logs it.
#[cfg(unix)]
#[test]
fn test_build_refuses_a_symlink_to_home_and_mounts_a_folder_in_it() {
    let repo = TestRepo::new();
    repo.create_markdown("index.md", "# Home\n");
    let home = common::visible_tempdir();
    std::fs::create_dir(home.path().join("Movies")).unwrap();
    std::fs::write(home.path().join("diary.png"), b"private").unwrap();
    std::fs::write(home.path().join("Movies/clip.mp4"), b"clip").unwrap();
    std::fs::create_dir(repo.path().join("static")).unwrap();
    std::os::unix::fs::symlink(home.path(), repo.path().join("static/home")).unwrap();
    std::os::unix::fs::symlink(
        home.path().join("Movies"),
        repo.path().join("static/videos"),
    )
    .unwrap();
    let out = TempDir::new().expect("temp output dir");

    let output = mbr_command()
        .env("HOME", home.path())
        .env("RUST_LOG", "mbr=info")
        .arg("-b")
        .arg("--output")
        .arg(out.path())
        .arg(repo.path())
        .output()
        .expect("failed to run the mbr binary");
    // `tracing`'s fmt layer writes to stdout; take both streams.
    let stderr = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        stderr_of(&output)
    );
    assert!(output.status.success(), "build failed:\n{stderr}");
    assert!(
        out.path().join("home").symlink_metadata().is_err(),
        "a link to $HOME must not be built"
    );
    assert_eq!(
        std::fs::read(out.path().join("videos/clip.mp4")).unwrap(),
        b"clip"
    );
    assert!(
        stderr.contains("home directory") && stderr.contains("Not serving"),
        "the refusal is logged with its reason:\n{stderr}"
    );
    assert!(
        stderr.contains("Serving external folder"),
        "the accepted mount is logged:\n{stderr}"
    );
}

/// The peer-site layout with `static/videos -> <elsewhere>`; returns the
/// content root and the temp dirs keeping it alive.
#[cfg(unix)]
fn mounted_peer_site() -> (std::path::PathBuf, TempDir, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let content = dir.path().join("site/content");
    let static_dir = dir.path().join("site/static");
    std::fs::create_dir_all(content.join(".mbr")).unwrap();
    std::fs::create_dir_all(static_dir.join("images")).unwrap();
    std::fs::write(
        content.join(".mbr/config.toml"),
        "static_folder = \"../static\"\n",
    )
    .unwrap();
    std::fs::write(content.join("index.md"), "# Home\n").unwrap();
    std::fs::write(static_dir.join("images/logo.png"), b"png").unwrap();
    let elsewhere = common::visible_tempdir();
    std::fs::write(elsewhere.path().join("clip.mp4"), b"clip").unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), static_dir.join("videos")).unwrap();
    (content, dir, elsewhere)
}

/// Builds `content` with `extra` arguments and environment; returns the
/// output dir and the combined log.
#[cfg(unix)]
fn build_with(content: &Path, args: &[&str], env: &[(&str, &str)]) -> (TempDir, String) {
    let out = TempDir::new().expect("temp output dir");
    let output = mbr_command()
        .envs(env.iter().copied())
        .env("RUST_LOG", "mbr=info")
        .arg("-b")
        .args(args)
        .arg("--output")
        .arg(out.path())
        .arg(content)
        .output()
        .expect("failed to run the mbr binary");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        stderr_of(&output)
    );
    assert!(output.status.success(), "build failed:\n{log}");
    (out, log)
}

/// `-b --no-symlink-mounts`: the mount is not built, and the build says the
/// switch is on. Without it, the same site places the file.
#[cfg(unix)]
#[test]
fn test_no_symlink_mounts_flag_skips_the_mount_in_a_build() {
    let (content, _dir, _elsewhere) = mounted_peer_site();
    let (out, _) = build_with(&content, &[], &[]);
    assert_eq!(
        std::fs::read(out.path().join("videos/clip.mp4")).unwrap(),
        b"clip"
    );

    let (out, log) = build_with(&content, &["--no-symlink-mounts"], &[]);
    assert!(out.path().join("videos").symlink_metadata().is_err());
    assert!(out.path().join("images/logo.png").exists());
    assert!(log.contains("Symlink mounts are disabled"), "{log}");
}

/// `MBR_NO_SYMLINK_MOUNTS=1` does the same — and the repository cannot undo
/// it: its own `.mbr/config.toml` asks for mounts in every spelling, and is
/// ignored.
#[cfg(unix)]
#[test]
fn test_no_symlink_mounts_env_var_wins_over_the_repository_config() {
    let (content, _dir, _elsewhere) = mounted_peer_site();
    std::fs::write(
        content.join(".mbr/config.toml"),
        "static_folder = \"../static\"\nno_symlink_mounts = false\nsymlink_mounts = true\n",
    )
    .unwrap();
    let (out, log) = build_with(&content, &[], &[("MBR_NO_SYMLINK_MOUNTS", "1")]);
    assert!(out.path().join("videos").symlink_metadata().is_err());
    assert!(log.contains("Symlink mounts are disabled"), "{log}");
    // And the same config.toml cannot switch mounts *off* either.
    std::fs::write(
        content.join(".mbr/config.toml"),
        "static_folder = \"../static\"\nno_symlink_mounts = true\nsymlink_mounts = false\n",
    )
    .unwrap();
    let (out, _) = build_with(&content, &[], &[]);
    assert_eq!(
        std::fs::read(out.path().join("videos/clip.mp4")).unwrap(),
        b"clip"
    );
}

/// `mbr -s`: the mounted file is served by default, and a 404 with
/// `--no-symlink-mounts` or with `MBR_NO_SYMLINK_MOUNTS=1`.
#[cfg(unix)]
#[test]
fn test_no_symlink_mounts_applies_to_the_server() {
    use std::io::{Read, Write};
    let (content, _dir, _elsewhere) = mounted_peer_site();
    let status = |port: u16, path: &str| -> Option<u16> {
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        )
        .ok()?;
        let mut response = String::new();
        stream.read_to_string(&mut response).ok()?;
        response.split_whitespace().nth(1)?.parse().ok()
    };
    for (env, flag, expected) in [
        (None, false, 200),
        (Some("1"), false, 404),
        (None, true, 404),
    ] {
        let port = common::find_available_port();
        let mut cmd = mbr_command();
        if let Some(value) = env {
            cmd.env("MBR_NO_SYMLINK_MOUNTS", value);
        }
        if flag {
            cmd.arg("--no-symlink-mounts");
        }
        let mut child = cmd
            .arg("-s")
            .arg("-q")
            .arg("-p")
            .arg(port.to_string())
            .arg(&content)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn mbr -s");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut answer = None;
        while std::time::Instant::now() < deadline {
            if status(port, "/images/logo.png") == Some(200) {
                answer = status(port, "/videos/clip.mp4");
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(
            answer,
            Some(expected),
            "MBR_NO_SYMLINK_MOUNTS={env:?}, --no-symlink-mounts: {flag}"
        );
    }
}
