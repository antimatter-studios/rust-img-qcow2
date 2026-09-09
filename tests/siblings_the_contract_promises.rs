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
fn sibling_of(path: &str) -> Option<String> {
    let rest = path.strip_prefix("../")?;
    let name = rest.split('/').next()?;
    if name.is_empty() || name == ".." {
        return None;
    }
    Some(name.to_string())
}

/// Every sibling `chores.yml` names anywhere — `sources:` entries and the
/// `cp` in `cmds:` alike.
///
/// Every scalar in the document rather than the two keys that carry one
/// today, because the question is what a reader of this file would know to
/// provision, and a task that mentions a directory tells them about it
/// wherever it does so.
fn siblings_the_contract_declares(chores: &str) -> BTreeSet<String> {
    let documents = Yaml::load_from_str(chores)
        .unwrap_or_else(|e| panic!("chores.yml does not parse as YAML: {e}"));

    let mut found = BTreeSet::new();
    for document in &documents {
        collect_siblings(document, &mut found);
    }
    found
}

fn collect_siblings(node: &Yaml, found: &mut BTreeSet<String>) {
    match node {
        Yaml::Value(_) => {
            if let Some(text) = node.as_str() {
                for start in text.match_indices("../").map(|(i, _)| i) {
                    if let Some(sibling) = sibling_of(&text[start..]) {
                        // Stop at anything that cannot be a directory name;
                        // `cp "../rust-fs-core/include/fs_core.h" x` is one
                        // scalar carrying a quote and a second argument.
                        let clean: String = sibling
                            .chars()
                            .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                            .collect();
                        if !clean.is_empty() {
                            found.insert(clean);
                        }
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
            for (key, value) in mapping {
                collect_siblings(key, found);
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

/// A path that does not leave the crate is not a sibling; reporting it
/// would make the check fire on something a consumer already has.
#[test]
fn a_path_inside_this_crate_is_not_a_sibling() {
    let needs = siblings_the_manifest_resolves(
        "[dependencies]\nam-vendored = { path = \"vendor/am-vendored\" }\n",
    );
    assert!(needs.is_empty(), "read {needs:?}");
}
