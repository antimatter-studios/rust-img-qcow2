//! `chores.yml:11` states the contract this crate publishes to its
//! consumers:
//!
//! > `staticlib` is the whole interface a consumer needs, AND IT TAKES NO
//! > OUTPUT DIRECTORY.
//!
//! A consumer reads that, provisions the siblings `chores.yml` names, and
//! runs `chore staticlib`. It was false for as long as `am-partitions` was
//! a path dev-dependency: cargo builds the resolve graph for EVERY command,
//! dev-dependencies included, so `cargo build --release` needed
//! `../rust-partitions` on disk exactly as `cargo test` does. Measured on
//! `aac5d3d` in a checkout carrying only `../rust-fs-core`:
//!
//! ```text
//! Caused by: failed to read .../rust-partitions/Cargo.toml
//! chore: staticlib: exit status 101
//! ```
//!
//! The error names a directory the contract says a consumer does not need.
//!
//! This refuses any path dependency on a sibling `chores.yml` does not
//! declare. It is deliberately not a list of allowed names: adding a
//! sibling stays possible, and costs one line in `sources:` — which is the
//! line a consumer provisions from, so the contract and the build move
//! together or not at all.
//!
//! It PARSES both files. `path` is legal in four dependency tables, under
//! `[target.'cfg(..)'.dependencies]`, and in `[patch]`; it can be an inline
//! table or its own section, with either quote style. A scan for
//! `path = "../` would miss most of those spellings and a guard that
//! misreads its input reports protection it is not providing.
//!
//! ## What this does NOT catch
//!
//! A registry dependency that pulls a SECOND copy of a crate this one
//! resolves by path. That is why `[patch.crates-io]` exists in
//! `Cargo.toml`, and the thing that catches it is CI building the `inspect`
//! example (`cargo build --locked --all-targets`) in a checkout holding
//! only `../rust-fs-core`. Two copies of `am-fs-core` give
//! `the trait `BlockRead` is not implemented for `Qcow2Reader``, not a
//! manifest that looks any different from a correct one.

use std::collections::BTreeSet;

use saphyr::{LoadableYamlNode, Yaml};

/// The tables in which cargo honours a `path`. `patch` and `replace` are
/// keyed by source rather than by dependency name, so their values are one
/// level deeper; `walk_for_paths` recurses, which covers both without
/// enumerating registries.
const MANIFEST_ROOTS: [&str; 6] = [
    "dependencies",
    "dev-dependencies",
    "build-dependencies",
    "target",
    "patch",
    "replace",
];

/// The directory name of every sibling a `path` in `text` resolves through,
/// e.g. `../rust-fs-core/foo` and `../rust-fs-core` both give
/// `rust-fs-core`.
///
/// A `path` that does not leave this crate (`path = "vendor/x"`) is not a
/// sibling and is not reported: the contract is about directories a
/// consumer has to provision beside the checkout.
fn siblings_the_manifest_resolves(manifest: &str) -> BTreeSet<String> {
    let parsed: toml::Table = manifest
        .parse()
        .unwrap_or_else(|e| panic!("Cargo.toml does not parse as TOML: {e}"));

    let mut found = BTreeSet::new();
    for root in MANIFEST_ROOTS {
        if let Some(node) = parsed.get(root) {
            walk_for_paths(node, &mut found);
        }
    }
    found
}

/// Recurses because the depth differs per table: `dependencies.x.path` is
/// two levels, `target.'cfg(unix)'.dependencies.x.path` is four, and
/// `patch.crates-io.x.path` is three. Depth is not the property being
/// tested — a `path` key with a string value is.
fn walk_for_paths(node: &toml::Value, found: &mut BTreeSet<String>) {
    let toml::Value::Table(table) = node else {
        return;
    };
    for (key, value) in table {
        if key == "path" {
            if let Some(sibling) = value.as_str().and_then(sibling_of) {
                found.insert(sibling);
            }
        }
        walk_for_paths(value, found);
    }
}

/// `../rust-fs-core/include` -> `rust-fs-core`. `None` for a path that
/// stays inside this crate.
///
/// NORMALISED, NOT PREFIX-TESTED. This was `strip_prefix("../")`, and
/// cargo does not require that spelling: `./../rust-partitions` and
/// `..//rust-partitions` resolve to the same directory and both build
/// (measured, cargo 1.94.1, `--offline`). Both returned `None` here, so
/// the sibling never reached `needs`, the difference against `declares`
/// was empty, and the guard passed on a manifest that still could not
/// build in the checkout `chores.yml:11` describes. #69 again, restored
/// by a spelling.
///
/// The direction of that miss is what makes it worth the change: on the
/// MANIFEST side a miss is a FALSE NEGATIVE — the guard is silent and the
/// build breaks in a consumer's checkout. (On the chores side the same
/// miss is a false positive, which is loud.)
///
/// A PATH THAT CLIMBS TWICE IS STILL A PREREQUISITE. `../../foo` is not a
/// sibling, and the old reader dropped it — silently, which is the shape
/// this function exists to stop. It is now named `../foo`: everything the
/// path keeps after leaving this crate's directory, joined. Both halves
/// normalise the same way, so a `chores.yml` that declares `../../foo`
/// produces the same name and the two still agree.
fn sibling_of(path: &str) -> Option<String> {
    // `.` and empty segments are noise: `a/./b` and `a//b` are `a/b`.
    let segments: Vec<&str> = path
        .split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    // NORMALISE THE WHOLE PATH, NOT ONLY ITS PREFIX. This counted the
    // leading `..` segments with `take_while`, so a climb that came
    // AFTER an ordinary segment was invisible: `subdir/../../rust-x`
    // resolves, exactly as cargo resolves it, to the sibling `rust-x`
    // -- `subdir` then back out of it, then out of the crate -- and the
    // prefix count returned 0 because the first segment is `subdir`.
    // The sibling never reached `needs`, `needs - declares` stayed
    // empty, and the guard passed on a manifest that still could not
    // build. That is the FALSE-NEGATIVE direction, which on this half
    // is the dangerous one.
    //
    // A `..` cancels the segment before it when there is one, and
    // otherwise climbs out of the crate. That is one rule and it
    // subsumes the prefix case.
    let mut climbs = 0usize;
    let mut inside: Vec<&str> = Vec::new();
    for segment in segments {
        if segment == ".." {
            if inside.pop().is_none() {
                climbs += 1;
            }
        } else {
            inside.push(segment);
        }
    }
    if climbs == 0 {
        return None;
    }
    // `../..` climbs but names no directory at all.
    let name = inside.first()?;
    // One climb is a sibling and is named plainly. More than one leaves
    // the siblings' directory too, and is named with the climbs it kept
    // so the two halves still produce the same string for the same path.
    Some(format!("{}{name}", "../".repeat(climbs - 1)))
}

/// Every sibling `chores.yml` DECLARES: the `sources:` entries and the
/// `cmds:` of each task, and nothing else.
///
/// THE TWO HALVES ARE NOT SYMMETRIC, AND THAT DECIDES THE READING.
/// `needs` is what the build requires; `declares` is what a consumer is
/// told to provide; the guard fires on `needs - declares`. So widening
/// `declares` moves the guard only toward NOT firing — it can never
/// raise a false alarm, only pass silently. Every scalar admitted here
/// is a channel through which the guard can be silenced for a name,
/// permanently, by text that provisions nothing.
///
/// This read every scalar in the document, mapping keys included, on
/// the argument that "a task that mentions a directory tells a reader
/// about it wherever it does so". That premise holds for `cmds:` — the
/// `cp "../rust-fs-core/include/fs_core.h"` genuinely is how this repo
/// declares that sibling, and a consumer reading the task learns to
/// provision it. It does not hold for a `desc:`, a `vars:` value, a
/// `generates:` entry or a mapping key: nobody provisions a directory
/// because a description mentioned one. Either of these silenced the
/// guard for `rust-partitions` on a manifest that could not build:
///
/// ```yaml
/// vars:
///   PARTITIONS_DIR: '../rust-partitions'   # used by nothing
/// ```
/// ```yaml
///   staticlib:
///     desc: Build the static library without ../rust-partitions
/// ```
///
/// `sources:` alone would be narrower still, and would discard the
/// reading above rather than keep it. Anyone widening this again should
/// have to say what the wider reading buys, because the cost is one
/// silencing channel per field.
fn siblings_the_contract_declares(chores: &str) -> BTreeSet<String> {
    let documents = Yaml::load_from_str(chores)
        .unwrap_or_else(|e| panic!("chores.yml does not parse as YAML: {e}"));

    let mut found = BTreeSet::new();
    for document in &documents {
        let Some(tasks) = document.as_mapping_get("tasks").and_then(Yaml::as_mapping) else {
            continue;
        };
        for (_, task) in tasks {
            for field in ["sources", "cmds"] {
                let Some(entries) = task.as_mapping_get(field).and_then(Yaml::as_sequence) else {
                    continue;
                };
                for entry in entries {
                    collect_siblings(entry, &mut found);
                }
            }
        }
    }
    found
}

/// The value of a leading `NAME=` shell assignment, or `None` if the
/// token is not one.
///
/// Only a token whose prefix is a legal shell variable name counts, so
/// `SRC=../rust-x` yields `../rust-x` while `--flag=../rust-x` and
/// `a=b=../rust-x` are left alone -- the first is an option rather than
/// an assignment, and the second is not a name the shell would accept.
/// Getting that wrong widens the reader, and on this half a wider
/// reader is a channel for silencing the guard.
fn shell_assignment_value(token: &str) -> Option<&str> {
    let (name, value) = token.split_once('=')?;
    let mut chars = name.chars();
    let first = chars.next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some(value)
}

/// The siblings named by one `sources:` or `cmds:` entry.
///
/// TOKENISED, NOT SCANNED FOR `../`. The old reader took every byte
/// offset of the literal `../` and trimmed the result at the first
/// character that could not be in a directory name. That anchor was a
/// second, independent narrowing beside `sibling_of`'s prefix test: a
/// path spelled `./../rust-x` reaches it only by accident of containing
/// `../` at byte 2, and one containing no literal `../` at all was
/// never offered. Splitting the scalar into shell-ish words and handing
/// each to `sibling_of` puts one normaliser in charge of both halves,
/// which is what makes the manifest side and this side agree.
///
/// A `cmds:` entry is shell, so quotes and redirections are stripped
/// from each word: `cp "../rust-fs-core/include/fs_core.h" "$OUT"` has
/// to yield `rust-fs-core`.
fn collect_siblings(node: &Yaml, found: &mut BTreeSet<String>) {
    match node {
        Yaml::Value(_) => {
            if let Some(text) = node.as_str() {
                for word in text.split_whitespace() {
                    let word = word.trim_matches(|c| c == '"' || c == '\'' || c == ';');
                    // A SHELL ASSIGNMENT CARRIES A PATH, and dropping
                    // it was a regression this tokeniser introduced.
                    // `SRC=../rust-fs-core/include; cp "$SRC/..." ...`
                    // genuinely requires the sibling, and the `../`
                    // scan this replaced found it. Split on whitespace,
                    // `SRC=../rust-fs-core/include` is one token whose
                    // first `/`-segment is `SRC=..`, so `sibling_of`
                    // refused it and the declaration was lost -- the
                    // guard then reported a provisioned sibling as
                    // undeclared. On this half that is the
                    // false-POSITIVE direction: loud, not silent, but
                    // still a guard refusing a correct file.
                    let word = shell_assignment_value(word).unwrap_or(word);
                    if let Some(sibling) = sibling_of(word) {
                        found.insert(sibling);
                    }
                }
            }
        }
        Yaml::Sequence(items) => {
            for item in items {
                collect_siblings(item, found);
            }
        }
        Yaml::Mapping(mapping) => {
            for (_, value) in mapping {
                collect_siblings(value, found);
            }
        }
        _ => {}
    }
}

fn manifest() -> String {
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("Cargo.toml is readable")
}

fn chores() -> String {
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/chores.yml"))
        .expect("chores.yml is readable")
}

// ---------------------------------------------------------------------
// The two halves. Both are needed: a reader that returned the empty set
// would satisfy the subset check without reading anything.
// ---------------------------------------------------------------------

/// THE CHECK. A path dependency on a sibling the contract does not name is
/// a prerequisite no consumer can discover.
#[test]
fn the_manifest_needs_no_sibling_the_contract_does_not_declare() {
    let needs = siblings_the_manifest_resolves(&manifest());
    let declares = siblings_the_contract_declares(&chores());

    let undeclared: Vec<&String> = needs.difference(&declares).collect();
    assert!(
        undeclared.is_empty(),
        "Cargo.toml resolves {undeclared:?} through a path, and chores.yml \
         names no such directory. chores.yml:11 promises `staticlib` is the \
         whole interface a consumer needs, so `chore staticlib` will exit \
         101 in the checkout that sentence describes. Either take the \
         dependency from the registry, or declare the directory in \
         chores.yml `sources:` so a consumer knows to provision it. \
         Manifest needs {needs:?}; chores.yml declares {declares:?}."
    );
}

/// The acceptance half, on the real files: the legal path dependency this
/// crate has always had must still be read, and read as legal.
#[test]
fn the_one_sibling_this_crate_does_need_is_read_and_allowed() {
    let needs = siblings_the_manifest_resolves(&manifest());
    assert!(
        needs.contains("rust-fs-core"),
        "am-fs-core is a path dependency on ../rust-fs-core; a reader that \
         does not see it sees nothing, and the check above would pass over \
         an empty set. Read {needs:?}"
    );
    assert!(
        siblings_the_contract_declares(&chores()).contains("rust-fs-core"),
        "chores.yml copies ../rust-fs-core/include/fs_core.h and lists it \
         under sources:; a reader that does not see it makes every path \
         dependency look undeclared"
    );
}

/// The defect, as it stood. Reintroducing the path dependency must be
/// caught rather than tolerated.
#[test]
fn a_path_dev_dependency_is_a_prerequisite_exactly_as_a_normal_one_is() {
    let manifest = r#"
[dependencies]
am-fs-core = { path = "../rust-fs-core", version = "0.2.10" }

[dev-dependencies]
am-partitions = { path = "../rust-partitions", version = "0.4" }
"#;
    let needs = siblings_the_manifest_resolves(manifest);
    assert!(needs.contains("rust-partitions"), "read {needs:?}");
    assert!(needs.contains("rust-fs-core"), "read {needs:?}");

    let declares = siblings_the_contract_declares(
        "tasks:\n  staticlib:\n    sources:\n      - '../rust-fs-core/include/fs_core.h'\n",
    );
    assert_eq!(
        needs.difference(&declares).collect::<Vec<_>>(),
        vec!["rust-partitions"],
        "the undeclared sibling is the one reported"
    );
}

/// The fix's own shape must read as clean, or the check would be arguing
/// with the remedy it asks for.
#[test]
fn a_registry_dependency_is_not_a_prerequisite() {
    let manifest = r#"
[dependencies]
am-fs-core = { path = "../rust-fs-core", version = "0.2.10" }

[dev-dependencies]
am-partitions = "0.4"

[patch.crates-io]
am-fs-core = { path = "../rust-fs-core" }
"#;
    let needs = siblings_the_manifest_resolves(manifest);
    assert_eq!(
        needs.iter().map(String::as_str).collect::<Vec<_>>(),
        vec!["rust-fs-core"],
        "a version requirement names no directory, and the patch names the \
         one already declared"
    );
}

/// `[patch]` is a dependency table like any other and its paths are
/// prerequisites like any other — it is a level deeper, which is why the
/// walk recurses instead of indexing.
#[test]
fn a_patch_section_path_is_a_prerequisite_too() {
    let needs = siblings_the_manifest_resolves(
        "[patch.crates-io]\nsomething = { path = \"../rust-somewhere-else\" }\n",
    );
    assert!(needs.contains("rust-somewhere-else"), "read {needs:?}");
}

/// Spellings a text scan would miss, all of them legal TOML for the same
/// thing.
#[test]
fn every_spelling_of_a_path_dependency_is_read() {
    let manifest = r#"
[dependencies.am-one]
path = '../rust-one'          # single quotes, own section, trailing comment

[target.'cfg(unix)'.dev-dependencies]
am-two = { version = "1", path = "../rust-two/sub/dir" }

[build-dependencies]
am-three = { path = "../rust-three" }
"#;
    let needs = siblings_the_manifest_resolves(manifest);
    assert_eq!(
        needs.iter().map(String::as_str).collect::<Vec<_>>(),
        vec!["rust-one", "rust-three", "rust-two"],
    );
}

/// #74 — CARGO DOES NOT REQUIRE THE CANONICAL SPELLING.
///
/// `sibling_of` tested `strip_prefix("../")`, and both of these build
/// (measured, cargo 1.94.1, `--offline`) while returning `None` from
/// it: the sibling never reached `needs`, the difference against
/// `declares` was empty, and the guard passed on a manifest that could
/// not build in the checkout `chores.yml:11` describes.
///
/// A MISS ON THIS SIDE IS A FALSE NEGATIVE — silent, and the build
/// breaks in a consumer's checkout rather than here.
#[test]
fn a_path_spelled_any_way_cargo_accepts_is_still_a_prerequisite() {
    for spelling in [
        "../rust-partitions",
        "./../rust-partitions",
        "..//rust-partitions",
        ".././rust-partitions",
        "../rust-partitions/",
        "../rust-partitions/include",
    ] {
        assert_eq!(
            sibling_of(spelling).as_deref(),
            Some("rust-partitions"),
            "{spelling} resolves to ../rust-partitions and needs that directory on disk"
        );
    }

    let manifest = r#"
[dependencies]
am-fs-core = { path = "../rust-fs-core", version = "0.2.10" }

[dev-dependencies]
am-partitions = { path = "./../rust-partitions", version = "0.4" }
"#;
    let needs = siblings_the_manifest_resolves(manifest);
    assert!(
        needs.contains("rust-partitions"),
        "the whole check turns on this reaching `needs`. Read {needs:?}"
    );
}

/// A PATH THAT CLIMBS TWICE IS STILL A PREREQUISITE, and the old reader
/// dropped it — silently, which is the shape this file exists to stop.
///
/// It is not a sibling, so it is named with the climbs it keeps rather
/// than pretending to be one. Both halves normalise the same way, so a
/// `chores.yml` declaring the same path produces the same string.
#[test]
fn a_path_that_climbs_past_the_siblings_is_reported_rather_than_dropped() {
    assert_eq!(sibling_of("../../shared").as_deref(), Some("../shared"));
    assert_eq!(sibling_of("../../../deep").as_deref(), Some("../../deep"));
    assert_eq!(sibling_of("../..").as_deref(), None, "names no directory");

    let manifest = "[dependencies]\nx = { path = \"../../shared\" }\n";
    let needs = siblings_the_manifest_resolves(manifest);
    assert!(needs.contains("../shared"), "read {needs:?}");

    let declares = siblings_the_contract_declares(
        "tasks:\n  staticlib:\n    sources:\n      - '../../shared/x.h'\n",
    );
    assert!(
        declares.contains("../shared"),
        "both halves must produce the same string for the same path: {declares:?}"
    );
}

/// #75 — A MENTION IS NOT A DECLARATION.
///
/// `declares` was every scalar in the document. Widening it moves the
/// guard only toward not firing, so each admitted field is a channel
/// that silences it for a name in exchange for nothing.
#[test]
fn a_directory_named_outside_sources_and_cmds_does_not_count_as_declared() {
    let unused_var = concat!(
        "vars:\n",
        "  PARTITIONS_DIR: '../rust-partitions'\n",
        "tasks:\n  staticlib:\n    sources:\n      - Cargo.toml\n",
    );
    let in_a_desc = concat!(
        "tasks:\n  staticlib:\n",
        "    desc: Build the static library without ../rust-partitions\n",
        "    sources:\n      - Cargo.toml\n",
    );
    let in_generates = concat!(
        "tasks:\n  staticlib:\n",
        "    generates:\n      - '../rust-partitions/dist/lib.a'\n",
        "    sources:\n      - Cargo.toml\n",
    );
    for (what, chores) in [
        ("an unused vars: value", unused_var),
        ("a desc:", in_a_desc),
        ("a generates: entry", in_generates),
    ] {
        let declares = siblings_the_contract_declares(chores);
        assert!(
            !declares.contains("rust-partitions"),
            "{what} provisions nothing; a consumer does not read it as a prerequisite. \
             Read {declares:?}"
        );
    }
}

/// THE ACCEPTANCE HALF OF THE SAME NARROWING, and it keeps the reading
/// this file argued for: the `cp` in `cmds:` IS how this repository
/// declares `../rust-fs-core`, and a consumer reading that task does
/// learn to provision it.
#[test]
fn a_sources_entry_and_a_cmds_argument_both_declare() {
    let from_sources = concat!(
        "tasks:\n  staticlib:\n",
        "    sources:\n      - '../rust-fs-core/include/fs_core.h'\n",
    );
    let from_cmds = concat!(
        "tasks:\n  staticlib:\n",
        "    cmds:\n      - 'cp \"../rust-fs-core/include/fs_core.h\" \"$OUT/include\"'\n",
    );
    for (what, chores) in [("sources:", from_sources), ("the cp in cmds:", from_cmds)] {
        assert!(
            siblings_the_contract_declares(chores).contains("rust-fs-core"),
            "{what} tells a consumer to provision the directory"
        );
    }

    // And on the real file, which carries it in BOTH — so this
    // repository stays green under any of the three readings, which is
    // why the narrowing had to be argued rather than measured here.
    let declares = siblings_the_contract_declares(&chores());
    assert!(declares.contains("rust-fs-core"), "read {declares:?}");
}

/// A CLIMB AFTER AN ORDINARY SEGMENT IS STILL A CLIMB.
///
/// `sibling_of` counted only the LEADING `..` segments, so a path that
/// entered a directory and climbed back out of it resolved to a sibling
/// for cargo and to nothing for the guard. On this half that is the
/// false-negative direction: the sibling never reaches `needs`,
/// `needs - declares` stays empty, and the guard passes on a manifest
/// that cannot build in the checkout `chores.yml` describes.
#[test]
fn a_climb_after_an_ordinary_segment_is_still_a_sibling() {
    for (path, expected) in [
        // `subdir` then out of it then out of the crate.
        ("subdir/../../rust-x", "rust-x"),
        ("./subdir/../../rust-x", "rust-x"),
        ("a/b/../../../rust-x", "rust-x"),
        // Two net climbs, so it keeps one `../` the way a leading
        // `../../` does -- the two halves must spell it identically.
        ("subdir/../../../rust-x", "../rust-x"),
    ] {
        let needs = siblings_the_manifest_resolves(&format!(
            "[dependencies]\namvx = {{ path = \"{path}\" }}\n"
        ));
        assert!(
            needs.contains(expected),
            "{path} resolves to {expected} for cargo, so the guard must need it. \
             Read {needs:?}"
        );
    }

    // AND THE OTHER DIRECTION: a climb that is cancelled does NOT
    // leave the crate, so it is not a sibling and reporting it would
    // make the guard fire on something a consumer already has.
    for path in ["subdir/../vendor/x", "a/b/../c", "./a/../b"] {
        let needs = siblings_the_manifest_resolves(&format!(
            "[dependencies]\namvx = {{ path = \"{path}\" }}\n"
        ));
        assert!(
            needs.is_empty(),
            "{path} stays inside the crate. Read {needs:?}"
        );
    }
}

/// A SHELL ASSIGNMENT IN `cmds:` DECLARES THE SIBLING IT NAMES, and
/// this is a REGRESSION TEST: the `../` scan that the tokeniser
/// replaced found this form, and the whitespace split lost it.
///
/// Direction matters. On the `declares` half a miss is a false
/// POSITIVE — the guard reports a sibling the contract does provision
/// as undeclared. Loud rather than silent, but still a guard refusing a
/// correct file, which is the failure mode this whole file warns about.
#[test]
fn a_path_carried_by_a_shell_assignment_still_declares() {
    let chores = concat!(
        "tasks:\n  staticlib:\n",
        "    cmds:\n",
        "      - 'SRC=../rust-fs-core/include; cp \"$SRC/fs_core.h\" \"$OUT/include\"'\n",
    );
    let declares = siblings_the_contract_declares(chores);
    assert!(
        declares.contains("rust-fs-core"),
        "the assignment names the directory the cp then reads, so a consumer learns to \
         provision it. Read {declares:?}"
    );

    // AND THE READER IS NOT WIDENED FURTHER THAN THAT. An option that
    // merely looks like an assignment is not one, and admitting it
    // would add a channel for silencing the guard in exchange for
    // nothing — the argument this file already makes about `desc:`.
    for (what, token) in [
        ("an option", "--include=../rust-partitions"),
        ("a name the shell would reject", "2SRC=../rust-partitions"),
        ("a doubled assignment", "a=b=../rust-partitions"),
    ] {
        let chores = format!(
            "tasks:\n  staticlib:\n    cmds:\n      - 'echo {token}'\n    sources:\n      - Cargo.toml\n"
        );
        let declares = siblings_the_contract_declares(&chores);
        assert!(
            !declares.contains("rust-partitions"),
            "{what} is not a shell assignment; it declares nothing. Read {declares:?}"
        );
    }
}

/// #77: THE TOKENISER SURVIVED REPLACEMENT BY A `match_indices("../")`
/// ANCHOR, AND THIS IS THE INPUT WHERE THE TWO READINGS DIFFER.
///
/// `match_indices("../")` matches TWICE inside `../../rust-fs-core`, so
/// the anchor reading collects `../rust-fs-core` from offset 0 and
/// `rust-fs-core` from offset 3. The second is a declaration nobody
/// wrote, and it is exactly the name the manifest needs — so
/// `needs - declares` comes out empty and the guard says nothing about
/// a sibling that is not provisioned. The tokeniser collects only
/// `../rust-fs-core`.
///
/// THE OBVIOUS WIDER REMEDY IS WRONG: do not make `sibling_of` reject
/// multi-climb paths.
/// [`a_path_that_climbs_past_the_siblings_is_reported_rather_than_dropped`]
/// exists to stop exactly that, and dropping such a path silently is
/// the defect this file was written for. What was missing was the test.
#[test]
fn a_twice_climbing_path_declares_only_what_it_names() {
    let chores = concat!(
        "tasks:\n  staticlib:\n",
        "    cmds:\n",
        "      - 'cp \"../../rust-fs-core/include/fs_core.h\" \"$OUT/include/fs_core.h\"'\n",
    );
    let declares = siblings_the_contract_declares(chores);
    assert!(
        declares.contains("../rust-fs-core"),
        "the path climbs twice, so it is spelled with the climb it kept. Read {declares:?}"
    );
    assert!(
        !declares.contains("rust-fs-core"),
        "nothing here declares the plain sibling `rust-fs-core`; reading one out of the \
         second `../` in `../../` invents a declaration and silences the guard for the \
         one name the manifest actually needs. Read {declares:?}"
    );
}

/// A path that does not leave the crate is not a sibling; reporting it
/// would make the check fire on something a consumer already has.
#[test]
fn a_path_inside_this_crate_is_not_a_sibling() {
    let needs = siblings_the_manifest_resolves(
        "[dependencies]\nam-vendored = { path = \"vendor/am-vendored\" }\n",
    );
    assert!(needs.is_empty(), "read {needs:?}");
}
