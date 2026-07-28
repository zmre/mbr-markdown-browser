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
