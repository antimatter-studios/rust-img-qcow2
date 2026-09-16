//! A test target gated on a feature is not run without that feature.
//!
//! `tests/qemu_validation.rs` opens with `#![cfg(feature = "qemu-validation")]`,
//! so without the feature the whole binary is empty. The default `test` job
//! runs `cargo test --locked --all-targets`, which built and ran it anyway,
//! and its log said
//!
//! ```text
//!      Running tests/qemu_validation.rs
//! test result: ok. 0 passed; 0 failed; 0 ignored; ... finished in 0.00s
//! ```
//!
//! a passing line, under the cross-validation suite's own name, for a run
//! that validated nothing. If the dedicated job were ever dropped, that line
//! would remain and nothing would say cross-validation had stopped.
//!
//! `required-features` on the target's `[[test]]` entry makes cargo skip it
//! when the feature is off, so the line is absent rather than green. This
//! PARSES `Cargo.toml` for that. The gate itself is found by a line scan of
//! each test file's inner attributes: there is no Rust parser here, and the
//! scan only has to recognise the one spelling this crate uses, which the
//! control below requires it to find.

use std::path::Path;

/// `(file stem, feature)` for every `tests/*.rs` whose crate is gated with
/// `#![cfg(feature = "...")]`.
fn feature_gated_test_targets(root: &Path) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let dir = root.join("tests");
    let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {dir:?}: {e}"));
    for entry in entries {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for line in text.lines() {
            let line = line.trim();
            if let Some(feature) = line
                .strip_prefix("#![cfg(feature = \"")
                .and_then(|rest| rest.strip_suffix("\")]"))
            {
                let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
                found.push((stem, feature.to_string()));
            }
        }
    }
    found.sort();
    found
}

/// The features `Cargo.toml` requires before building the test target `name`.
fn required_features(manifest: &toml::Table, name: &str) -> Vec<String> {
    manifest
        .get("test")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|t| t.get("name").and_then(toml::Value::as_str) == Some(name))
        .flat_map(|t| {
            t.get("required-features")
                .and_then(toml::Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|f| f.as_str().map(str::to_string))
        .collect()
}

#[test]
fn a_feature_gated_test_target_requires_its_feature() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let gated = feature_gated_test_targets(root);

    // CONTROL: the scan must find the target this test exists for. Without
    // it, a scan that matched nothing would pass on every manifest.
    assert!(
        gated
            .iter()
            .any(|(stem, feature)| stem == "qemu_validation" && feature == "qemu-validation"),
        "expected tests/qemu_validation.rs to be found gated on qemu-validation; found {gated:?}"
    );

    let manifest: toml::Table = std::fs::read_to_string(root.join("Cargo.toml"))
        .unwrap()
        .parse()
        .expect("Cargo.toml parses as TOML");
    for (stem, feature) in &gated {
        let required = required_features(&manifest, stem);
        assert!(
            required.contains(feature),
            "tests/{stem}.rs is compiled only with feature `{feature}`, but Cargo.toml's \
             [[test]] entry for `{stem}` requires {required:?}; without it `cargo test \
             --all-targets` runs an empty binary and reports `ok`"
        );
    }
}
