//! `scripts/tier.sh` resolves the canonical output-budget wrapper, or says so.
//!
//! # WHAT CHANGED, AND WHY THIS FILE CHANGED WITH IT
//!
//! This repository used to carry `scripts/output-budget.sh`, a committed copy
//! of somebody else's script, and this file held that copy to the five
//! promises every test tier here rests on. The copy is gone. The canonical
//! script lives in `rust-fs-core` and `scripts/tier.sh` resolves it at
//! RUNTIME, so there is exactly one of it in the family instead of one per
//! repository, each internally consistent and none compared with any other.
//!
//! Pinning that script's BEHAVIOUR from here would now be testing another
//! repository's file from the wrong repository: rust-fs-core owns those five
//! promises and tests them where they are written, and a second set of
//! assertions here would only tell us when core had deliberately changed
//! something. What this repository owns, and what is untested anywhere else,
//! is the RESOLVER -- so that is what this file guards:
//!
//! 1. a tier runs its command through a wrapper `tier.sh` went and found; a
//!    pass is quiet and names its log, and a failure hands on the COMMAND's
//!    status rather than the wrapper's;
//! 2. a tier whose budget is breached exits 65, which is a status of its own
//!    so "too loud" is never mistaken for "red";
//! 3. the borrowed copy is taken for the run and given back afterwards;
//! 4. a core that is not there is REFUSED, loudly, naming what would supply
//!    it -- it does not fall back to anything;
//! 5. a core that is there but answers `--version` with the wrong string is
//!    refused just as loudly, because a present-but-wrong copy is the failure
//!    mode a fallback would hide;
//! 6. the committed copy stays deleted.
//!
//! # WHY THE CONTRACT IS A VERSION STRING AND NOT A DIGEST
//!
//! The obvious way to prove a resolved file is the right file is to pin its
//! SHA-256. It was rejected. A digest recorded in every consuming repository
//! has to be updated in every consuming repository the moment core edits a
//! comment in that script -- seven repositories moving in lockstep for a
//! change none of them made, which is precisely the coupling deleting the
//! vendored copy was meant to remove. The script prints
//! `rust-fs-core-output-budget 1` for `--version`, that string is the
//! contract, and it moves when the interface moves rather than when the bytes
//! do.
//!
//! # NOTHING HERE SKIPS
//!
//! Every tier in this repository goes through `tier.sh`, so a host that
//! cannot resolve the wrapper cannot run the suite. These tests therefore
//! FAIL rather than skip when it cannot be found, and the failure quotes
//! `tier.sh`'s own message, which names the sibling path it looked in, the
//! version string it wanted and the minimum rust-fs-core release that carries
//! the script. A check that quietly passes when its subject is absent reports
//! protection it is not providing.
//!
//! The other half of the arrangement -- that every tier in `ci.yml` and
//! `chores.yml` actually GOES through `tier.sh`, under a non-zero budget,
//! with the two files agreeing on the numbers -- is in `tests/ci_profile.rs`,
//! which is the file that already parses both.

use std::path::PathBuf;
use std::process::{Command, Output};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The `bash` that can actually run a shell script.
///
/// # `bash` ON PATH IS NOT BASH ON A WINDOWS RUNNER
///
/// `C:\Windows\System32\bash.exe` is the WSL launcher, it ships with the
/// operating system, and `System32` comes early in `PATH` -- so
/// `Command::new("bash")` finds it before Git Bash. With no WSL
/// distribution installed it prints nothing useful and exits 1, which is
/// how every test in this file failed on `windows-latest` while the same
/// scripts ran perfectly in the workflow: an Actions step that says
/// `shell: bash` is handed Git Bash by name and never consults `PATH`.
///
/// So this asks for Git Bash by name on Windows and falls back to `PATH`
/// elsewhere -- and, if that file is not there, still falls back to `PATH`
/// rather than deciding the host cannot run the suite.
fn bash() -> PathBuf {
    if cfg!(windows) {
        let git_bash = PathBuf::from(r"C:\Program Files\Git\bin\bash.exe");
        if git_bash.is_file() {
            return git_bash;
        }
    }
    PathBuf::from("bash")
}

/// Run `scripts/tier.sh` from the repository root.
///
/// FROM THE ROOT, WITH RELATIVE PATHS, because this suite runs on
/// windows-latest, where `bash` is Git Bash and an absolute Windows path
/// handed to it as an argument is a path with backslashes in it. A relative
/// path plus a working directory is the one spelling that means the same
/// thing on all three runners -- and it is the same reason `tier.sh` prefers
/// the sibling checkout over anything `cargo metadata` reports.
///
/// `core_root` sets `FS_CORE_ROOT`, which `tier.sh` treats as authoritative:
/// when it is set, that directory is the only place the wrapper is looked
/// for. `None` leaves the environment alone so the resolver does what it
/// would do for a person running `chore test`.
fn run_tier(arguments: &[&str], core_root: Option<&str>, verbose: bool) -> Output {
    let mut command = Command::new(bash());
    command
        .current_dir(repo())
        .arg("scripts/tier.sh")
        .args(arguments);
    // `None` leaves the variable alone rather than removing it: a tier here
    // must resolve the wrapper exactly the way `chore test` and ci.yml do, and
    // ci.yml sets FS_CORE_ROOT for the whole job.
    if let Some(root) = core_root {
        command.env("FS_CORE_ROOT", root);
    }
    command.env("CLI_ARGS", if verbose { " --verbose " } else { " " });
    command.output().unwrap_or_else(|e| {
        panic!(
            "could not run `{} scripts/tier.sh`: {e}. Every test tier in this \
             repository runs through that script, so a host without `bash` \
             cannot run this suite -- which is why this is a failure and not \
             a skip. On Windows, Git Bash provides it.",
            bash().display()
        )
    })
}

fn stdout_and_stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string() + &String::from_utf8_lossy(&output.stderr)
}

/// Fail with `tier.sh`'s own diagnosis when it could not find a wrapper.
///
/// The message it prints already names the sibling path, the expected
/// `--version` string and the minimum rust-fs-core release, so quoting it is
/// the most useful thing a failure here can say.
fn assert_resolved(output: &Output, what: &str) {
    let printed = stdout_and_stderr(output);
    assert!(
        !printed.contains("The output budget wrapper is rust-fs-core's"),
        "{what} could not resolve the canonical wrapper, so this suite cannot \
         run. This is a failure and not a skip. tier.sh said:\n{printed}"
    );
}

/// A scratch directory under `tmp/`, which is gitignored, named relative to
/// the repository root so it can be handed to Git Bash unchanged.
fn scratch(name: &str) -> String {
    let relative = format!("tmp/resolver/{name}");
    let directory = repo().join(&relative);
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory)
        .unwrap_or_else(|e| panic!("could not create {relative}: {e}"));
    relative
}

fn log_contents(log_name: &str) -> String {
    let path = repo()
        .join("tmp")
        .join("logs")
        .join(format!("{log_name}.log"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("the tier left no log at {}: {e}", path.display()))
}

/// A command that prints 40 lines and succeeds.
const LOUD: &str = "i=0; while [ $i -lt 40 ]; do echo line $i; i=$((i+1)); done";

#[test]
fn a_passing_tier_is_quiet_and_names_the_log_it_kept_everything_in() {
    let output = run_tier(
        &[
            "quiet",
            "resolver-quiet",
            "100",
            "9000",
            "--",
            "echo",
            "hello",
        ],
        None,
        false,
    );
    assert_resolved(&output, "a passing tier");

    let printed = stdout_and_stderr(&output);
    assert!(
        output.status.success(),
        "a tier inside its budget failed:\n{printed}"
    );
    assert!(
        !printed.contains("hello"),
        "the tier put its command's output on the terminal, which is the whole \
         thing this arrangement exists to stop:\n{printed}"
    );
    assert!(
        printed.contains("quiet: ok"),
        "the tier printed no verdict line, so the reader is told nothing at \
         all -- quiet is not the same as silent:\n{printed}"
    );
    assert!(
        printed.contains("resolver-quiet.log"),
        "the verdict line does not name the log, so the output that was \
         withheld cannot be found:\n{printed}"
    );
    assert_eq!(
        log_contents("resolver-quiet").trim(),
        "hello",
        "the log did not keep the command's output. The budget is not a gag: \
         everything goes to the log whatever happens to the terminal."
    );
}

#[test]
fn a_failing_tier_hands_on_the_commands_own_status_and_names_the_log() {
    let output = run_tier(
        &[
            "bad",
            "resolver-bad",
            "100",
            "9000",
            "--",
            "sh",
            "-c",
            "echo the reason; exit 7",
        ],
        None,
        false,
    );
    assert_resolved(&output, "a failing tier");

    assert_eq!(
        output.status.code(),
        Some(7),
        "a failing tier must exit with the COMMAND's status, not the \
         wrapper's and not the copy's. This is the `cargo test | tee log` \
         defect -- a red suite reading green because something else in the \
         pipeline succeeded."
    );
    let printed = stdout_and_stderr(&output);
    assert!(
        printed.contains("FAILED (exit 7)") && printed.contains("resolver-bad.log"),
        "a failure said neither what status it got nor where the run is. The \
         canonical wrapper prints no tail unless asked (OUTPUT_BUDGET_FAIL_TAIL, \
         or --tail), so that one line is the whole of what a reader gets \
         before fetching the log:\n{printed}"
    );
    assert!(
        log_contents("resolver-bad").contains("the reason"),
        "the failing run's output did not reach the log"
    );
}

#[test]
fn a_tier_over_its_budget_exits_65_and_still_keeps_the_whole_log() {
    let output = run_tier(
        &["loud", "resolver-loud", "5", "0", "--", "sh", "-c", LOUD],
        None,
        false,
    );
    assert_resolved(&output, "a tier over its budget");

    assert_eq!(
        output.status.code(),
        Some(65),
        "a tier over its line budget must exit 65 -- a status of its own, so a \
         suite that printed too much is not mistaken for a suite that failed. \
         It exited {:?}:\n{}",
        output.status.code(),
        stdout_and_stderr(&output)
    );
    assert_eq!(
        log_contents("resolver-loud").lines().count(),
        40,
        "a breached budget lost the output"
    );
}

#[test]
fn the_wrapper_is_borrowed_for_the_run_and_given_back_afterwards() {
    // The copy exists so a tier cannot have its wrapper changed underneath it
    // by somebody moving the rust-fs-core checkout mid-run. The command below
    // lists the copy WHILE it is running -- its output goes to the log, not
    // the terminal -- and the file it names must be gone once the tier ends.
    let output = run_tier(
        &[
            "borrow",
            "resolver-borrow",
            "100",
            "9000",
            "--",
            "sh",
            "-c",
            "test -f \"$TIER_OUTPUT_BUDGET\" && echo \"$TIER_OUTPUT_BUDGET\"",
        ],
        None,
        false,
    );
    assert_resolved(&output, "a tier");
    assert!(
        output.status.success(),
        "the wrapper tier.sh named in TIER_OUTPUT_BUDGET was not on disk while \
         the tier was running, so the run did not get its own copy:\n{}",
        stdout_and_stderr(&output)
    );

    let borrowed = log_contents("resolver-borrow").trim().to_string();
    assert!(
        borrowed.contains("output-budget."),
        "the tier named no copy at all; TIER_OUTPUT_BUDGET was `{borrowed}`"
    );
    assert!(
        !repo().join(&borrowed).exists(),
        "{borrowed} outlived the run. The EXIT trap in tier.sh is what removes \
         it, and `exec`ing the wrapper would silently defeat that."
    );
}

#[test]
fn verbose_reaches_the_wrapper_under_its_new_name() {
    // THE RENAME THAT FAILS SILENTLY. The canonical script reads
    // OUTPUT_BUDGET_VERBOSE; the vendored copy read FLTH_VERBOSE. Setting the
    // old name does not error -- the run simply stays quiet -- so the only
    // thing that can catch tier.sh exporting the wrong one is a test that
    // asks for a verbose run and looks for the output.
    let output = run_tier(
        &[
            "v",
            "resolver-verbose",
            "100",
            "9000",
            "--",
            "echo",
            "streamed",
        ],
        None,
        true,
    );
    assert_resolved(&output, "a verbose tier");

    let printed = stdout_and_stderr(&output);
    assert!(
        printed.contains("streamed"),
        "`--verbose` in CLI_ARGS did not stream the run. tier.sh maps it onto \
         OUTPUT_BUDGET_VERBOSE; the old FLTH_VERBOSE is not read by the \
         canonical wrapper, and setting it changes nothing at all:\n{printed}"
    );
}

#[test]
fn the_resolver_refuses_a_core_that_is_not_there() {
    let empty = scratch("no-core-here");
    let output = run_tier(
        &[
            "absent",
            "resolver-absent",
            "100",
            "9000",
            "--",
            "echo",
            "hi",
        ],
        Some(&empty),
        false,
    );

    assert_eq!(
        output.status.code(),
        Some(1),
        "a tier pointed at a directory with no wrapper in it did not refuse. \
         There is nothing to fall back TO -- the vendored copy is deleted -- so \
         anything other than a refusal means a tier ran unbudgeted:\n{}",
        stdout_and_stderr(&output)
    );

    let printed = stdout_and_stderr(&output);
    for expected in [
        "rust-fs-core-output-budget 1",
        "rust-fs-core",
        "v0.2.13",
        "FS_CORE_ROOT",
    ] {
        assert!(
            printed.contains(expected),
            "the refusal does not mention `{expected}`, so a reader is told it \
             failed without being told what would fix it:\n{printed}"
        );
    }
}

#[test]
fn the_resolver_refuses_a_core_whose_version_string_is_wrong() {
    // A PRESENT-BUT-WRONG COPY IS THE INTERESTING CASE. An absent one is
    // obvious; a file at the right path that answers `--version` with
    // something else is a different script wearing the right name, and a
    // resolver that shrugged and tried the next source would run it or run
    // something else without saying which.
    let wrong = scratch("wrong-version");
    let scripts = repo().join(&wrong).join("scripts");
    std::fs::create_dir_all(&scripts).expect("could not create the fake core's scripts/");
    std::fs::write(
        scripts.join("output-budget.sh"),
        "#!/usr/bin/env bash\necho 'some-other-budget 4'\n",
    )
    .expect("could not write the fake wrapper");

    let output = run_tier(
        &["wrong", "resolver-wrong", "100", "9000", "--", "echo", "hi"],
        Some(&wrong),
        false,
    );

    assert_eq!(
        output.status.code(),
        Some(1),
        "a wrapper answering --version with the wrong string was accepted:\n{}",
        stdout_and_stderr(&output)
    );
    let printed = stdout_and_stderr(&output);
    assert!(
        printed.contains("some-other-budget 4") && printed.contains("rust-fs-core-output-budget 1"),
        "the refusal did not print both what it got and what it wanted, which \
         is the whole diagnosis:\n{printed}"
    );
}

#[test]
fn the_vendored_copy_stays_deleted() {
    // Restoring it would work, quietly, for exactly as long as it took core's
    // script to change -- which is the drift this repository had before and
    // could not see. If a copy is ever needed again it is a decision, not a
    // file that reappears.
    let copy = repo().join("scripts").join("output-budget.sh");
    assert!(
        !copy.exists(),
        "{} is back. The canonical wrapper belongs to rust-fs-core and \
         tier.sh resolves it at runtime; a copy here is a copy that drifts.",
        copy.display()
    );
}
