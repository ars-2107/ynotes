//! Engine-level tests for the Phase 0 store: a note survives a
//! capture -> save -> reload round-trip, and store discovery works via both
//! the upward walk and the `$YNOTES_DIR` override.

use ynotes::{LineRange, Note, Scope, SelectorBundle, SourceFile, Store};

/// Builds a note describing lines 2..=3 of a small file with the given body.
///
/// All callers share the same target, scope and range, so two notes built
/// here differ only by `body` — exactly the "same location, edited context"
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

    let reloaded = store.notes_for("src/app.rs").expect("notes_for");
    assert_eq!(reloaded.len(), 1);
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
    // adopted as a store — that would write into someone else's tree.
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

    let notes = store.notes_for("src/app.rs").expect("notes_for");
    assert_eq!(notes.len(), 1, "no duplicate from a retried save");
}

#[test]
fn unknown_target_yields_no_notes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::init(dir.path()).expect("init");
    assert!(store.notes_for("nope.rs").expect("notes_for").is_empty());
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
    let (created, superseded) = store
        .save_superseding(&second)
        .expect("save_superseding second");

    assert!(created, "the revised note is newly written");
    assert_eq!(
        superseded, 1,
        "the earlier note at this location is retired"
    );

    let notes = store.notes_for("src/app.rs").expect("notes_for");
    assert_eq!(notes.len(), 1, "only the latest note survives");
    assert_eq!(notes[0], second, "the survivor is the revised note");
}

/// Two spellings of the same physical file — `sub/file.txt` and
/// `sub/../sub/file.txt` — must produce the same target string. The id is a
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

#[test]
fn save_superseding_is_a_no_op_for_identical_content() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Store::init(dir.path()).expect("init");

    let note = sample_note();
    store.save(&note).expect("save");

    // Re-saving identical content: the id matches, so nothing is written and
    // nothing else sits at that location to supersede.
    let (created, superseded) = store
        .save_superseding(&sample_note())
        .expect("save_superseding retry");
    assert!(!created, "an identical re-save creates nothing");
    assert_eq!(superseded, 0, "an identical re-save supersedes nothing");

    let notes = store.notes_for("src/app.rs").expect("notes_for");
    assert_eq!(notes.len(), 1, "still exactly one note");
}
