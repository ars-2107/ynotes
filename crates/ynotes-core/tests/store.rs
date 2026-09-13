//! Store round-trips, record integrity, discovery, and index recovery.

use ynotes::{IdMatch, LineRange, Note, Scope, SelectorBundle, SourceFile, Store};

/// Builds a note describing lines 2..=3 of a small file with the given body.
///
/// All callers share the same target, scope and range, so two notes built
/// here differ only by `body`, exactly the "same location, edited context"
/// case `save_superseding` must collapse.
fn note_with_body(body: &str) -> Note {
    let file = SourceFile::from_lines(
        "src/app.rs".into(),
        vec![
            "fn main() {".to_owned(),
            "    let x = 1;".to_owned(),
            "    let y = 2;".to_owned(),
            "}".to_owned(),
        ],
    );
    let range = LineRange::new(2, 3).expect("valid range");
    let bundle = SelectorBundle::capture(&file, range).expect("capture");
    Note::new(
        "src/app.rs".to_owned(),
        Scope::Range,
        bundle,
        body.to_owned(),
    )
    .expect("note")
}

/// Builds a note describing lines 2..=3 of a small file.
fn sample_note() -> Note {
    note_with_body("the body of main, the interesting part")
}

#[test]
fn note_survives_save_and_reload() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::init(dir.path()).expect("init");

    let note = sample_note();
    store.save(&note).expect("save");

    let reloaded = store.notes_for("src/app.rs").expect("notes_for").notes;
    assert_eq!(reloaded.len(), 1);
    assert_eq!(reloaded[0].bundle.schema, 1);
    assert_eq!(reloaded[0], note);
    assert_eq!(
        reloaded[0].bundle.quote.exact,
        "    let x = 1;\n    let y = 2;"
    );
}

#[test]
fn init_twice_is_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let first = Store::init(dir.path()).expect("first init");
    let second = Store::init(dir.path()).expect("second init must succeed (idempotent)");
    assert_eq!(first.root(), second.root());
}

#[test]
fn init_refuses_a_dotynotes_directory_without_head() {
    let dir = tempfile::tempdir().expect("tempdir");
    // A `.ynotes` directory left over from something unrelated must not be
    // adopted as a store, that would write into someone else's tree.
    std::fs::create_dir(dir.path().join(".ynotes")).expect("create stray .ynotes");
    let err = Store::init(dir.path()).expect_err("must refuse a non-store .ynotes");
    assert!(matches!(err, ynotes::Error::Invalid(_)));
}

#[test]
fn discover_walks_up_from_a_nested_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::init(dir.path()).expect("init");

    let nested = dir.path().join("a").join("b").join("c");
    std::fs::create_dir_all(&nested).expect("nested dirs");

    let found = Store::discover_from(&nested).expect("discover from nested");
    assert_eq!(found.root(), store.root());
}

#[test]
fn open_rejects_a_directory_without_head() {
    let dir = tempfile::tempdir().expect("tempdir");
    let err = Store::open(dir.path()).expect_err("not a store");
    assert!(matches!(err, ynotes::Error::Invalid(_)));
}

#[test]
fn discover_from_reports_not_found_when_absent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let err = Store::discover_from(dir.path()).expect_err("no store");
    assert!(matches!(err, ynotes::Error::StoreNotFound { .. }));
}

#[test]
fn saving_identical_context_twice_is_idempotent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::init(dir.path()).expect("init");

    // Two independently constructed notes with identical content: the
    // content-addressed id makes them the same note.
    let first = sample_note();
    let second = sample_note();
    assert_eq!(first.id, second.id, "identical content ⇒ identical id");

    assert!(store.save(&first).expect("first save"), "first is new");
    assert!(
        !store.save(&second).expect("second save"),
        "retry is a no-op"
    );

    let notes = store.notes_for("src/app.rs").expect("notes_for").notes;
    assert_eq!(notes.len(), 1, "no duplicate from a retried save");
}

#[test]
fn unknown_target_yields_no_notes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::init(dir.path()).expect("init");
    assert!(
        store
            .notes_for("nope.rs")
            .expect("notes_for")
            .notes
            .is_empty()
    );
}

#[test]
fn save_superseding_replaces_an_earlier_note_at_the_same_location() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::init(dir.path()).expect("init");

    let first = note_with_body("the first take on this region");
    let second = note_with_body("a revised take on the same region");
    // Same target/scope/range, different body ⇒ different content-addressed id.
    assert_ne!(first.id, second.id, "a changed body must change the id");

    store.save(&first).expect("save first");
    let outcome = store
        .save_superseding(&second)
        .expect("save_superseding second");

    assert!(outcome.created, "the revised note is newly written");
    assert_eq!(
        outcome.superseded, 1,
        "the earlier note at this location is retired"
    );
    assert!(
        outcome.neighbours.is_empty(),
        "the note it replaced is superseded, not reported as a neighbour"
    );

    let notes = store.notes_for("src/app.rs").expect("notes_for").notes;
    assert_eq!(notes.len(), 1, "only the latest note survives");
    assert_eq!(notes[0], second, "the survivor is the revised note");
}

/// Two spellings of the same physical file, `sub/file.txt` and
/// `sub/../sub/file.txt`, must produce the same target string. The id is a
/// content hash of `(target, scope, bundle, body)` (invariant #6), so target
/// normalisation is what makes the id deterministic across spellings. Without
/// this the same file would carry duplicate notes under cosmetically different
/// keys.
#[test]
fn relativize_collapses_parent_dir_segments() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("sub")).expect("mkdir sub");
    std::fs::write(dir.path().join("sub/file.txt"), "hello\n").expect("write");
    let store = Store::init(dir.path()).expect("init");

    let clean = store
        .relativize(&dir.path().join("sub/file.txt"))
        .expect("clean");
    let with_parent = store
        .relativize(&dir.path().join("sub/../sub/file.txt"))
        .expect("with parent");
    assert_eq!(
        clean, with_parent,
        "`..`-segments must collapse before strip-prefix",
    );
    assert_eq!(clean, "sub/file.txt");
}

/// macOS exposes the same temp directory under `/tmp` (a symlink) and
/// `/private/tmp` (the canonical form). A `relativize` that compared the two
/// without canonicalising would reject a `/tmp/...` input as outside a
/// `/private/tmp/...` work tree even though both name the same place.
#[cfg(target_os = "macos")]
#[test]
fn relativize_resolves_a_symlinked_work_tree_root() {
    let dir = tempfile::Builder::new()
        .prefix("ynotes-symlink-")
        .tempdir_in("/tmp")
        .expect("tempdir in /tmp");
    std::fs::create_dir_all(dir.path().join("sub")).expect("mkdir");
    std::fs::write(dir.path().join("sub/file.txt"), "hello\n").expect("write");
    let store = Store::init(dir.path()).expect("init");

    // `dir.path()` is canonical (`/private/tmp/...`). Build a `/tmp/...`
    // spelling by stripping the `/private` prefix.
    let via_tmp = dir
        .path()
        .strip_prefix("/private")
        .map_or_else(
            |_| dir.path().to_path_buf(),
            |p| std::path::Path::new("/").join(p),
        )
        .join("sub/file.txt");
    let via_private = dir.path().join("sub/file.txt");

    let a = store.relativize(&via_tmp).expect("/tmp spelling");
    let b = store
        .relativize(&via_private)
        .expect("/private/tmp spelling");
    assert_eq!(a, b, "`/tmp` and `/private/tmp` spellings must collapse");
    assert_eq!(a, "sub/file.txt");
}

/// `match_id_prefix` collapses the raw prefix scan into the three outcomes the
/// by-id commands branch on: a unique hit, an ambiguous prefix, and no match.
/// Two notes differing only in body carry distinct content-hash ids, so a full
/// id selects exactly one, the empty prefix (which every id starts with) is
/// ambiguous across both, and a hex string matching neither resolves to
/// `None`, the same 0/1/many a front-end renders as proceed / "ambiguous" /
/// "no match".
#[test]
fn match_id_prefix_distinguishes_one_ambiguous_none() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::init(dir.path()).expect("init");

    let first = note_with_body("the first take on this region");
    let second = note_with_body("a revised take on the same region");
    assert_ne!(first.id, second.id, "different bodies ⇒ different ids");
    store.save(&first).expect("save first");
    store.save(&second).expect("save second");

    // A full id is unique to its own note.
    match store.match_id_prefix(&first.id).expect("match full id") {
        IdMatch::One(note) => assert_eq!(note.id, first.id),
        other => panic!("expected One, got {other:?}"),
    }

    // Every id starts with the empty prefix, so it matches both notes.
    match store.match_id_prefix("").expect("match empty prefix") {
        IdMatch::Ambiguous(candidates) => {
            let got: std::collections::BTreeSet<String> =
                candidates.iter().map(|n| n.id.clone()).collect();
            let want: std::collections::BTreeSet<String> =
                [first.id.clone(), second.id.clone()].into_iter().collect();
            assert_eq!(got, want, "both notes are candidates");
        }
        other => panic!("expected Ambiguous, got {other:?}"),
    }

    // A 64-hex string that is not a stored id (all-`f` cannot collide with a
    // real SHA-256 of the fixtures) matches nothing.
    let absent = "f".repeat(64);
    assert!(matches!(
        store.match_id_prefix(&absent).expect("match absent"),
        IdMatch::None
    ));
}

/// The well-formedness policy for an id prefix is engine-owned: a sub-4-char
/// prefix and a non-hex string are both rejected as
/// [`ynotes::Error::InvalidIdPrefix`], a usage-class error every front-end
/// maps to its own input-error surface, rather than resolving to a misleading
/// "no match". The matcher itself stays pure (see the empty-prefix probe in
/// the test above), so validation is an explicit, separate step.
#[test]
fn validate_id_prefix_rejects_short_and_non_hex_prefixes() {
    let err = ynotes::validate_id_prefix("ab").expect_err("sub-4-char prefix");
    assert!(matches!(err, ynotes::Error::InvalidIdPrefix(_)));
    assert!(err.to_string().contains("too short"));

    let err = ynotes::validate_id_prefix("abcz").expect_err("non-hex prefix");
    assert!(matches!(err, ynotes::Error::InvalidIdPrefix(_)));
    assert!(err.to_string().contains("not a valid note id"));

    ynotes::validate_id_prefix("abcd").expect("4 hex chars is well-formed");
}

/// Ids are always emitted lowercase, so an uppercase hex prefix could never
/// match a stored id; validation refuses it up front as
/// [`ynotes::Error::InvalidIdPrefix`] rather than letting it dead-end as a
/// misleading "no match".
#[test]
fn validate_id_prefix_rejects_uppercase_hex() {
    let err = ynotes::validate_id_prefix("DEADBEEF").expect_err("uppercase hex prefix");
    assert!(matches!(err, ynotes::Error::InvalidIdPrefix(_)));
}

#[test]
fn save_superseding_is_a_no_op_for_identical_content() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::init(dir.path()).expect("init");

    let note = sample_note();
    store.save(&note).expect("save");

    // Re-saving identical content: the id matches, so nothing is written and
    // nothing else sits at that location to supersede.
    let outcome = store
        .save_superseding(&sample_note())
        .expect("save_superseding retry");
    assert!(!outcome.created, "an identical re-save creates nothing");
    assert_eq!(
        outcome.superseded, 0,
        "an identical re-save supersedes nothing"
    );
    assert!(
        outcome.neighbours.is_empty(),
        "a note is never its own neighbour"
    );

    let notes = store.notes_for("src/app.rs").expect("notes_for").notes;
    assert_eq!(notes.len(), 1, "still exactly one note");
}

#[test]
fn discover_or_init_creates_store_at_git_root() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap(); // a git repo
    let sub = dir.path().join("src");
    std::fs::create_dir(&sub).unwrap();
    let (store, created) = ynotes::Store::discover_or_init_from(&sub).unwrap();
    assert!(created);
    assert!(dir.path().join(".ynotes").is_dir());
    // Canonicalise both sides: `tempdir()` hands back a non-canonical path
    // (e.g. macOS's `/var` symlink to `/private/var`), and `workdir()` returns
    // the git root verbatim, so a raw `==` would fail on that spelling gap.
    assert_eq!(
        store.workdir().unwrap().canonicalize().unwrap(),
        dir.path().canonicalize().unwrap()
    );
}

#[test]
fn discover_or_init_reuses_an_existing_store_without_creating() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    ynotes::Store::init(dir.path()).unwrap();
    let (_, created) = ynotes::Store::discover_or_init_from(dir.path()).unwrap();
    assert!(!created);
}

#[test]
fn discover_or_init_without_git_propagates_store_not_found() {
    let dir = tempfile::tempdir().unwrap(); // no .git
    let err = ynotes::Store::discover_or_init_from(dir.path()).unwrap_err();
    assert!(matches!(err, ynotes::Error::StoreNotFound { .. }));
}

#[test]
fn discover_or_init_treats_a_git_file_as_a_root() {
    // git worktrees have `.git` as a FILE, not a directory.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".git"), "gitdir: elsewhere\n").unwrap();
    let (_, created) = ynotes::Store::discover_or_init_from(dir.path()).unwrap();
    assert!(created);
}

/// `$YNOTES_DIR` wins over both discovery and the git-root bootstrap,
/// mirroring `discover()`'s precedence: with the override pointing at a valid
/// store elsewhere, `discover_or_init_from` inside a git repo with no store
/// returns the override store (`created == false`) and creates nothing in the
/// repo, an explicit override is never a creation site. `temp-env` scopes
/// the mutation (set, run, restore, under a global lock), and nextest runs
/// each test in its own process anyway.
#[test]
fn discover_or_init_honours_ynotes_dir_and_never_creates_there() {
    let store_dir = tempfile::tempdir().unwrap();
    let store = Store::init(store_dir.path()).unwrap();

    let git_dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(git_dir.path().join(".git")).unwrap();

    temp_env::with_var("YNOTES_DIR", Some(store.root().as_os_str()), || {
        let (found, created) = ynotes::Store::discover_or_init_from(git_dir.path()).unwrap();
        assert!(!created, "an explicit override never creates a store");
        assert_eq!(found.root(), store.root(), "the override store is returned");
        assert!(
            !git_dir.path().join(".ynotes").exists(),
            "no store is conjured in the git repo while the override is set"
        );
    });
}

/// The store keeps managed lines in `.ynotes/.gitattributes` so a committed
/// store merges without per-clone setup. That is hygiene, not store state: it
/// used to run as a hard precondition on every mutation, so a `.gitattributes`
/// the process cannot maintain failed the *delete* as well, blocking the one
/// command that gets rid of a bad note. `doctor` reports the gap
/// (`gitattributes_missing`), so the repair degrades instead of refusing.
#[test]
fn a_mutation_survives_an_unmaintainable_gitattributes() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let note = note_with_body("why this is deliberate");
    store.save(&note).unwrap();

    // A directory where the file belongs: readable as a path, impossible to
    // rewrite as text, and portable across platforms.
    let attrs = store.root().join(".gitattributes");
    std::fs::remove_file(&attrs).unwrap();
    std::fs::create_dir(&attrs).unwrap();

    store
        .remove(&note)
        .expect("deleting a note must not depend on git-integration hygiene");
    assert!(
        store.notes_for("src/app.rs").unwrap().notes.is_empty(),
        "the delete actually happened"
    );

    // The gap is not swallowed: it stays visible where it is actionable.
    let health = store.health().unwrap();
    assert!(
        !health.gitattributes_missing.is_empty(),
        "doctor must still report the managed lines as missing"
    );
}

/// Superseding collapses notes at an *identical* location only. A second note a
/// few lines away is a different location, so it is written happily beside the
/// first, which is exactly how a store fills with restatements of the same
/// fact and stops being worth reading. The save that creates the near-duplicate
/// is the last moment anyone can cheaply notice, so it reports what the file
/// already said.
#[test]
fn a_save_reports_the_other_notes_already_on_the_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::init(dir.path()).expect("init");

    let file = SourceFile::from_lines(
        "src/app.rs".into(),
        vec![
            "fn main() {".to_owned(),
            "    let x = 1;".to_owned(),
            "    let y = 2;".to_owned(),
            "    let z = 3;".to_owned(),
            "}".to_owned(),
        ],
    );
    let note_at = |from: u32, to: u32, body: &str| {
        let range = LineRange::new(from, to).expect("range");
        let bundle = SelectorBundle::capture(&file, range).expect("capture");
        Note::new(
            "src/app.rs".to_owned(),
            Scope::Range,
            bundle,
            body.to_owned(),
        )
        .expect("note")
    };

    let first = note_at(2, 2, "x is one because the protocol counts from one");
    assert!(
        store
            .save_superseding(&first)
            .expect("save first")
            .neighbours
            .is_empty(),
        "the first note on a file has no neighbours"
    );

    // A different location, so nothing is superseded, and without the report
    // this near-duplicate lands in total silence.
    let second = note_at(3, 3, "y is two for the same protocol reason");
    let outcome = store.save_superseding(&second).expect("save second");
    assert!(outcome.created);
    assert_eq!(
        outcome.superseded, 0,
        "a different range supersedes nothing"
    );
    assert_eq!(
        outcome.neighbours.len(),
        1,
        "the writer must be told the file already carries a note"
    );
    assert_eq!(outcome.neighbours[0].id, first.id);
    assert!(
        outcome.neighbours[0]
            .body
            .contains("protocol counts from one"),
        "the neighbour carries enough body to recognise a restatement"
    );

    // Earliest region first, so the same store always reports the same way.
    let third = note_at(4, 4, "z is three");
    let outcome = store.save_superseding(&third).expect("save third");
    let ranges: Vec<u32> = outcome
        .neighbours
        .iter()
        .map(|n| n.bundle.position.range.start())
        .collect();
    assert_eq!(ranges, vec![2, 3], "neighbours are ordered by region");
}
