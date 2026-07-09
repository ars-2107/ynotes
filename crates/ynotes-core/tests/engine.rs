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

#[test]
fn code_not_found_renders_the_reread_recovery() {
    let err = ynotes::Error::CodeNotFound;
    insta::assert_snapshot!(err.to_string(), @"the quoted code is not in this file; re-read the file and quote the region as it reads now");
}

#[test]
fn code_ambiguous_renders_every_candidate_and_the_diagnosis() {
    let err = ynotes::Error::CodeAmbiguous {
        lines: vec![61, 118, 204],
    };
    insta::assert_snapshot!(err.to_string(), @"the quoted code occurs 3 times (lines 61, 118, 204) and start matches none of them; correct start, or quote more surrounding lines");
}
