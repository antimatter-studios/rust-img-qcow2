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
        let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
        let features =
            crate_feature_gates(&text).unwrap_or_else(|line| panic!("tests/{stem}.rs: {line}"));
        found.extend(features.into_iter().map(|f| (stem.clone(), f)));
    }
    found.sort();
    found
}

/// The features a file's crate-level `#![cfg(feature = "...")]` lines
/// require.
///
/// A LINE SCAN, SO IT REFUSES WHAT IT CANNOT READ. Any other crate-level
/// `cfg` that mentions a feature -- `#![cfg(all(feature = "x"))]`,
/// `#![cfg(any(...))]`, a different spacing -- is an `Err` naming the
/// line, not a silent skip: skipped, that target would escape the check
/// while the control on `qemu_validation` still passed.
fn crate_feature_gates(text: &str) -> Result<Vec<String>, String> {
    let mut features = Vec::new();
    // Inner attributes precede every item, so the scan stops at the first
    // line that is not blank, a comment or an inner attribute. That also
    // keeps it out of string literals further down that happen to start
    // with `#![`, such as the ones in this file.
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if !line.starts_with("#![") {
            break;
        }
        if let Some(feature) = line
            .strip_prefix("#![cfg(feature = \"")
            .and_then(|rest| rest.strip_suffix("\")]"))
            .filter(|f| !f.contains('"'))
        {
            features.push(feature.to_string());
        } else if line.contains("feature") {
            return Err(format!(
                "unsupported crate-level feature gate `{line}`; spell it \
                 #![cfg(feature = \"...\")] or extend this check"
            ));
        }
    }
    Ok(features)
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

/// The scan's own two halves: the one spelling it reads, and every other
/// crate-level feature gate refused rather than skipped.
#[test]
fn a_crate_level_feature_gate_is_read_or_refused() {
    assert_eq!(
        crate_feature_gates("//! doc\n#![cfg(feature = \"qemu-validation\")]\nmod x;\n"),
        Ok(vec!["qemu-validation".to_string()])
    );
    assert_eq!(
        crate_feature_gates("#![allow(dead_code)]\nfn f() {}\n"),
        Ok(vec![])
    );
    // After the first item a line is code, not a crate attribute.
    assert_eq!(
        crate_feature_gates(
            "fn f() {}\nconst S: &str = \"\n#![cfg(all(feature = \\\"x\\\"))]\";\n"
        ),
        Ok(vec![])
    );
    for line in [
        "#![cfg(all(feature = \"qemu-validation\"))]",
        "#![cfg(any(feature = \"a\", feature = \"b\"))]",
        "#![cfg(feature=\"qemu-validation\")]",
        "#![cfg_attr(feature = \"x\", allow(dead_code))]",
    ] {
        assert!(
            crate_feature_gates(line).is_err(),
            "`{line}` is a feature gate the scan cannot read; it must be refused"
        );
    }
}
