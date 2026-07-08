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

#[test]
fn resolve_scope_rejects_past_eof_range() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.txt");
    std::fs::write(&f, "one\ntwo\n").unwrap();
    let source = ynotes::SourceFile::read(&f).unwrap();
    let spec = "1:9".parse::<ynotes::LineSpec>().unwrap();
    let err = ynotes::resolve_scope(spec, &source).unwrap_err();
    assert!(matches!(err, ynotes::Error::InvalidLocation(_)));
    assert!(err.to_string().contains("past the end"));
}

#[test]
fn resolve_scope_whole_file_is_file_scope() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.txt");
    std::fs::write(&f, "one\ntwo\n").unwrap();
    let source = ynotes::SourceFile::read(&f).unwrap();
    let (scope, range) = ynotes::resolve_scope("".parse().unwrap(), &source).unwrap();
    assert!(matches!(scope, ynotes::Scope::File));
    assert_eq!((range.start(), range.end()), (1, 2));
}
