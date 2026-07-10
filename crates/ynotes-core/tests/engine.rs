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

/// A `SourceFile` built directly from text, for tests that never touch disk.
fn src(text: &str) -> ynotes::SourceFile {
    ynotes::SourceFile::from_lines(
        std::path::PathBuf::from("mem.rs"),
        text.lines().map(str::to_owned).collect(),
    )
}

// --- the §4.4 rule -------------------------------------------------------

#[test]
fn verified_quote_at_start_wins_even_when_it_collides_elsewhere() {
    let s = src("one\ntwin\ntwin-tail\nfour\ntwin\ntwin-tail\nseven");
    let (scope, range) = ynotes::resolve_scope_verified(
        "5:6".parse::<ynotes::LineSpec>().unwrap(),
        "twin\ntwin-tail",
        &s,
    )
    .unwrap();
    assert!(matches!(scope, ynotes::Scope::Range));
    assert_eq!((range.start(), range.end()), (5, 6));
}

#[test]
fn verified_quote_relocates_down_when_start_is_stale() {
    let s = src("one\ntwo\nthree\nfour\nfive\nsix");
    let (scope, range) =
        ynotes::resolve_scope_verified("2".parse::<ynotes::LineSpec>().unwrap(), "five\nsix", &s)
            .unwrap();
    assert!(matches!(scope, ynotes::Scope::Range));
    assert_eq!((range.start(), range.end()), (5, 6));
}

#[test]
fn verified_quote_relocates_up_when_start_is_stale() {
    let s = src("one\ntwo\nthree\nfour\nfive\nsix");
    let (_scope, range) =
        ynotes::resolve_scope_verified("5".parse::<ynotes::LineSpec>().unwrap(), "two\nthree", &s)
            .unwrap();
    assert_eq!((range.start(), range.end()), (2, 3));
}

#[test]
fn verified_missing_quote_is_code_not_found() {
    let s = src("one\ntwo");
    let err = ynotes::resolve_scope_verified("1".parse::<ynotes::LineSpec>().unwrap(), "three", &s)
        .unwrap_err();
    assert!(matches!(err, ynotes::Error::CodeNotFound));
}

#[test]
fn verified_colliding_quote_with_stale_start_lists_every_candidate() {
    let s = src("dup\nx\ndup\ny");
    let err = ynotes::resolve_scope_verified("4".parse::<ynotes::LineSpec>().unwrap(), "dup", &s)
        .unwrap_err();
    match err {
        ynotes::Error::CodeAmbiguous { lines } => assert_eq!(lines, vec![1, 3]),
        other => panic!("expected CodeAmbiguous, got {other:?}"),
    }
}

#[test]
fn verified_whole_file_spec_is_a_usage_error() {
    let s = src("one\ntwo");
    let err = ynotes::resolve_scope_verified(ynotes::LineSpec::Whole, "one", &s).unwrap_err();
    assert!(matches!(err, ynotes::Error::InvalidLocation(_)));
    assert!(
        err.to_string().contains("requires a starting line"),
        "got: {err}"
    );
}

// --- scope mapping (§4.7) -------------------------------------------------

#[test]
fn verified_scope_is_line_for_a_one_line_quote_and_range_otherwise() {
    let s = src("one\ntwo\nthree");
    let (scope, range) =
        ynotes::resolve_scope_verified("2".parse::<ynotes::LineSpec>().unwrap(), "two", &s)
            .unwrap();
    assert!(matches!(scope, ynotes::Scope::Line));
    assert_eq!((range.start(), range.end()), (2, 2));

    let (scope, _) =
        ynotes::resolve_scope_verified("2".parse::<ynotes::LineSpec>().unwrap(), "two\nthree", &s)
            .unwrap();
    assert!(matches!(scope, ynotes::Scope::Range));

    // resolve_scope's inherited wart, preserved: a declared range is a range
    // note even when degenerate ("2:2"), while bare "2" is a line note.
    let (scope, _) =
        ynotes::resolve_scope_verified("2:2".parse::<ynotes::LineSpec>().unwrap(), "two", &s)
            .unwrap();
    assert!(matches!(scope, ynotes::Scope::Range));
}

// --- the span rule (§4.6) -------------------------------------------------

#[test]
fn verified_declared_end_fixes_the_span_beyond_a_short_quote() {
    // Opening-lines usage: quote the first two lines, declare the full span.
    let s = src("one\ntwo\nthree\nfour\nfive");
    let (_, range) = ynotes::resolve_scope_verified(
        "2:5".parse::<ynotes::LineSpec>().unwrap(),
        "two\nthree",
        &s,
    )
    .unwrap();
    assert_eq!((range.start(), range.end()), (2, 5));
}

#[test]
fn verified_overhanging_quote_disambiguates_without_widening_the_span() {
    // "dup / dup-tail" occurs twice; only the four-line quote is unique. The
    // overhang locates the region; the declared span (2 lines) still bounds it.
    let s = src("x\ndup\ndup-tail\ny\ny2\ndup\ndup-tail\nunique-a\nunique-b");
    let (_, range) = ynotes::resolve_scope_verified(
        "2:3".parse::<ynotes::LineSpec>().unwrap(),
        "dup\ndup-tail\nunique-a\nunique-b",
        &s,
    )
    .unwrap();
    assert_eq!((range.start(), range.end()), (6, 7));
}

#[test]
fn verified_span_past_eof_is_the_past_eof_usage_error() {
    let s = src("one\ntwo\nthree\nfour\nfive");
    let err = ynotes::resolve_scope_verified(
        "2:9".parse::<ynotes::LineSpec>().unwrap(),
        "four\nfive",
        &s,
    )
    .unwrap_err();
    assert!(matches!(err, ynotes::Error::InvalidLocation(_)));
    assert!(
        err.to_string().contains("past the end of the file"),
        "got: {err}"
    );
}

#[test]
fn verified_end_u32_max_reports_past_eof_without_overflow() {
    let s = src("one\ntwo");
    let spec = ynotes::LineSpec::Range(ynotes::LineRange::new(1, u32::MAX).unwrap());
    let err = ynotes::resolve_scope_verified(spec, "one", &s).unwrap_err();
    assert!(matches!(err, ynotes::Error::InvalidLocation(_)));
    assert!(
        err.to_string().contains("past the end of the file"),
        "got: {err}"
    );
}

#[test]
fn verified_line_zero_is_a_usage_error_not_an_underflow() {
    // Line(0) is unreachable through the parser but constructible directly.
    let s = src("one\ntwo");
    let err = ynotes::resolve_scope_verified(ynotes::LineSpec::Line(0), "one", &s).unwrap_err();
    assert!(matches!(err, ynotes::Error::InvalidLocation(_)));
}

// --- normalisation (§4.3) -------------------------------------------------

#[test]
fn verified_flush_left_quote_matches_an_indented_block() {
    let s = src("fn a() {\n    inner();\n    tail();\n}");
    let (_, range) = ynotes::resolve_scope_verified(
        "2".parse::<ynotes::LineSpec>().unwrap(),
        "inner();\ntail();",
        &s,
    )
    .unwrap();
    assert_eq!((range.start(), range.end()), (2, 3));
}

#[test]
fn verified_over_indented_quote_matches() {
    let s = src("fn a() {\n    inner();\n}");
    let (_, range) = ynotes::resolve_scope_verified(
        "2".parse::<ynotes::LineSpec>().unwrap(),
        "      inner();",
        &s,
    )
    .unwrap();
    assert_eq!((range.start(), range.end()), (2, 2));
}

#[test]
fn verified_space_quote_matches_a_tab_file_and_the_reverse() {
    // The regression a dedent rule would have shipped (spec §11): the indent
    // character must not matter, only the text.
    let tabs = src("fn a() {\n\tinner();\n\ttail();\n}");
    let (_, range) = ynotes::resolve_scope_verified(
        "2".parse::<ynotes::LineSpec>().unwrap(),
        "    inner();\n    tail();",
        &tabs,
    )
    .unwrap();
    assert_eq!((range.start(), range.end()), (2, 3));

    let spaces = src("fn a() {\n    inner();\n    tail();\n}");
    let (_, range) = ynotes::resolve_scope_verified(
        "2".parse::<ynotes::LineSpec>().unwrap(),
        "\tinner();\n\ttail();",
        &spaces,
    )
    .unwrap();
    assert_eq!((range.start(), range.end()), (2, 3));
}

#[test]
fn verified_trailing_whitespace_differences_normalise_away() {
    let s = src("one\ntwo   \nthree");
    let (_, range) =
        ynotes::resolve_scope_verified("2".parse::<ynotes::LineSpec>().unwrap(), "two\t", &s)
            .unwrap();
    assert_eq!((range.start(), range.end()), (2, 2));
}

#[test]
fn verified_blank_edge_lines_are_dropped_and_start_advances_past_the_lead() {
    // start names the quote's first line as written; the blank lead is
    // dropped and start advances with it, so a truthful quote that opens
    // blank corroborates instead of spuriously relocating.
    let s = src("one\ntwo\nthree\nfour");
    let (_, range) =
        ynotes::resolve_scope_verified("2".parse::<ynotes::LineSpec>().unwrap(), "\nthree\n\n", &s)
            .unwrap();
    assert_eq!((range.start(), range.end()), (3, 3));
}

#[test]
fn verified_interior_blank_lines_stay_significant() {
    let with_blank = src("one\nfoo\n\nbar");
    let (_, range) = ynotes::resolve_scope_verified(
        "2".parse::<ynotes::LineSpec>().unwrap(),
        "foo\n\nbar",
        &with_blank,
    )
    .unwrap();
    assert_eq!((range.start(), range.end()), (2, 4));

    let without_blank = src("one\nfoo\nmid\nbar");
    let err = ynotes::resolve_scope_verified(
        "2".parse::<ynotes::LineSpec>().unwrap(),
        "foo\n\nbar",
        &without_blank,
    )
    .unwrap_err();
    assert!(matches!(err, ynotes::Error::CodeNotFound));
}

#[test]
fn verified_blank_lead_shrinks_a_declared_span() {
    let s = src("one\ntwo\nthree\nfour\nfive");
    // Declared 2:4 with one blank lead: the region is 3:4, not 3:5.
    let (_, range) =
        ynotes::resolve_scope_verified("2:4".parse::<ynotes::LineSpec>().unwrap(), "\nthree", &s)
            .unwrap();
    assert_eq!((range.start(), range.end()), (3, 4));

    // A span the blank lead collapses entirely is a usage error.
    let err =
        ynotes::resolve_scope_verified("2:2".parse::<ynotes::LineSpec>().unwrap(), "\nthree", &s)
            .unwrap_err();
    assert!(matches!(err, ynotes::Error::InvalidLocation(_)));
}

#[test]
fn verified_crlf_file_matches_an_lf_quote() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("crlf.txt");
    std::fs::write(&f, "one\r\ntwo\r\nthree\r\n").unwrap();
    let source = ynotes::SourceFile::read(&f).unwrap();
    let (_, range) = ynotes::resolve_scope_verified(
        "2".parse::<ynotes::LineSpec>().unwrap(),
        "two\nthree",
        &source,
    )
    .unwrap();
    assert_eq!((range.start(), range.end()), (2, 3));
}

#[test]
fn verified_empty_or_blank_code_is_a_usage_error_not_an_engine_error() {
    let s = src("one\ntwo");
    for code in ["", "   \n\n\t"] {
        let err =
            ynotes::resolve_scope_verified("1".parse::<ynotes::LineSpec>().unwrap(), code, &s)
                .unwrap_err();
        // InvalidLocation (usage, exit 2) — not Invalid, which reads as
        // corrupt store data and exits 1.
        assert!(
            matches!(err, ynotes::Error::InvalidLocation(_)),
            "{code:?}: {err:?}"
        );
    }
}

#[test]
fn verified_quote_longer_than_the_file_is_code_not_found() {
    let s = src("one\ntwo");
    let err = ynotes::resolve_scope_verified(
        "1".parse::<ynotes::LineSpec>().unwrap(),
        "one\ntwo\nthree\nfour",
        &s,
    )
    .unwrap_err();
    assert!(matches!(err, ynotes::Error::CodeNotFound));
}

/// A real on-disk nine-line file, for the id keystones: `capture_full` walks
/// the filesystem (git discovery), so these need a genuine path. The bare
/// `.git/` directory pins discovery to the tempdir — hermetic whatever
/// repository the test runner itself sits in.
fn nine_line_file() -> (tempfile::TempDir, ynotes::SourceFile) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    let f = dir.path().join("a.rs");
    std::fs::write(&f, "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\n").unwrap();
    let source = ynotes::SourceFile::read(&f).unwrap();
    (dir, source)
}

/// Build the note a save would store for this resolution, in the same git
/// state, so ids are comparable.
fn note_for(
    source: &ynotes::SourceFile,
    scope: ynotes::Scope,
    range: ynotes::LineRange,
) -> ynotes::Note {
    let bundle = ynotes::SelectorBundle::capture_full(source, range).unwrap();
    ynotes::Note::new("a.rs".to_owned(), scope, bundle, "why".to_owned()).unwrap()
}

/// Keystone (spec §8): a verified line save and a bare coordinate save of the
/// same line hash to the same note id — same file, same git state. This
/// proves NOTHING across commits: `capture_full` folds the HEAD commit into
/// the bundle when the file is tracked, so ids rotate with history by design.
#[test]
fn verified_and_bare_line_saves_share_an_id() {
    let (_dir, source) = nine_line_file();
    let (vs, vr) =
        ynotes::resolve_scope_verified("5".parse::<ynotes::LineSpec>().unwrap(), "five", &source)
            .unwrap();
    let (bs, br) =
        ynotes::resolve_scope("5".parse::<ynotes::LineSpec>().unwrap(), &source).unwrap();
    assert_eq!((vs, vr), (bs, br));
    assert_eq!(note_for(&source, vs, vr).id, note_for(&source, bs, br).id);
}

/// Keystone (spec §8): `{start: 5, code: <five lines>}` is the same note as
/// `{start: 5, end: 9}`. Same-commit only; see the sibling test's caveat.
#[test]
fn verified_and_bare_range_saves_share_an_id() {
    let (_dir, source) = nine_line_file();
    let (vs, vr) = ynotes::resolve_scope_verified(
        "5".parse::<ynotes::LineSpec>().unwrap(),
        "five\nsix\nseven\neight\nnine",
        &source,
    )
    .unwrap();
    let (bs, br) =
        ynotes::resolve_scope("5:9".parse::<ynotes::LineSpec>().unwrap(), &source).unwrap();
    assert_eq!((vs, vr), (bs, br));
    assert_eq!(note_for(&source, vs, vr).id, note_for(&source, bs, br).id);
}

/// Keystone (spec §8): a save whose stale coordinates the quote corrected is
/// byte-identical — same id — to the save a correct caller would have made
/// with fresh coordinates. An agent that retries with the corrected range
/// gets `created: false`, not a duplicate. Same-commit only, as above.
#[test]
fn relocated_save_shares_an_id_with_fresh_coordinates() {
    let (_dir, source) = nine_line_file();
    let (vs, vr) = ynotes::resolve_scope_verified(
        "2".parse::<ynotes::LineSpec>().unwrap(),
        "five\nsix",
        &source,
    )
    .unwrap();
    assert_eq!((vr.start(), vr.end()), (5, 6));
    let (bs, br) =
        ynotes::resolve_scope("5:6".parse::<ynotes::LineSpec>().unwrap(), &source).unwrap();
    assert_eq!((vs, vr), (bs, br));
    assert_eq!(note_for(&source, vs, vr).id, note_for(&source, bs, br).id);
}
