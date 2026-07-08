//! Engine-level tests against the `ynotes` library API.
//!
//! `insta` inline snapshots (`@"…"`) keep the expected value next to the
//! assertion and need no `.snap` file; run `cargo insta review` after changing
//! rendered output to update them.

#[test]
fn invalid_error_renders_stable_message() {
    let err = ynotes::Error::Invalid("expected a line range".to_owned());
    insta::assert_snapshot!(err.to_string(), @"invalid ynotes data: expected a line range");
}

#[test]
fn version_is_non_empty() {
    assert!(!ynotes::version().is_empty());
}
