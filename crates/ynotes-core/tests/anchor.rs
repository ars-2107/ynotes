//! Phase 1 engine tests: the quote rung relocates a moved region, and — the
//! product's core promise — a region that vanishes is surfaced as orphaned,
//! never silently dropped.

use ynotes::{
    AnchorStatus, LineRange, LineSpec, Note, Scope, SelectorBundle, SourceFile, Store, query,
    resolve,
};

fn file(path: &str, lines: &[&str]) -> SourceFile {
    SourceFile::from_lines(path.into(), lines.iter().map(|s| (*s).to_owned()).collect())
}

#[test]
fn quote_rung_relocates_a_region_that_moved_down() {
    let v1 = file("a.rs", &["use std;", "fn target() {", "    work();", "}"]);
    let range = LineRange::new(2, 3).unwrap();
    let bundle = SelectorBundle::capture(&v1, range).unwrap();

    // Two lines inserted above the region: it is now at 4:5.
    let v2 = file(
        "a.rs",
        &[
            "// header",
            "// header 2",
            "use std;",
            "fn target() {",
            "    work();",
            "}",
        ],
    );
    let res = resolve(&bundle, &v2, None);

    assert_eq!(res.range, Some(LineRange::new(4, 5).unwrap()));
    assert!(matches!(res.status, AnchorStatus::Drifted { .. }));
}

#[test]
fn unique_quote_with_unchanged_file_is_anchored() {
    let f = file("a.rs", &["one", "two", "three", "four"]);
    let bundle = SelectorBundle::capture(&f, LineRange::new(2, 3).unwrap()).unwrap();
    let res = resolve(&bundle, &f, None);
    assert_eq!(res.range, Some(LineRange::new(2, 3).unwrap()));
    assert!(matches!(res.status, AnchorStatus::Anchored));
}

#[test]
fn a_vanished_region_is_orphaned_not_dropped() {
    let v1 = file("a.rs", &["alpha", "BEACON_ONE", "BEACON_TWO", "omega"]);
    let bundle = SelectorBundle::capture(&v1, LineRange::new(2, 3).unwrap()).unwrap();

    // Same line count, entirely different content: no real locator can find it.
    let v2 = file("a.rs", &["nothing", "like", "the", "original"]);
    let res = resolve(&bundle, &v2, None);

    assert_eq!(res.range, None);
    match res.status {
        AnchorStatus::Orphaned { last_known } => {
            assert_eq!(last_known, LineRange::new(2, 3).unwrap());
        }
        other => panic!("expected orphaned, got {other:?}"),
    }
}

#[test]
fn orphaned_notes_are_returned_even_for_a_non_overlapping_query() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();

    // Save a note for lines 2:3 of a file...
    let target = dir.path().join("code.rs");
    std::fs::write(&target, "alpha\nBEACON_ONE\nBEACON_TWO\nomega\n").unwrap();
    let src = SourceFile::read(&target).unwrap();
    let bundle = SelectorBundle::capture(&src, LineRange::new(2, 3).unwrap()).unwrap();
    let note = Note::new(
        store.relativize(&target).unwrap(),
        Scope::Range,
        bundle,
        "why these two lines matter".to_owned(),
    )
    .unwrap();
    store.save(&note).unwrap();

    // ...then rewrite the file so the region is gone, and query a line range
    // that does not overlap the old position at all.
    std::fs::write(&target, "x\ny\nz\nw\n").unwrap();
    let result = query(
        &store,
        &target,
        LineSpec::Range(LineRange::new(40, 50).unwrap()),
    )
    .unwrap();

    assert!(result.matched.is_empty());
    assert_eq!(
        result.orphaned.len(),
        1,
        "orphan must survive a disjoint query"
    );
    assert_eq!(result.orphaned[0].note.body, "why these two lines matter");
}

#[test]
fn fuzzy_does_not_false_positive_on_an_unrelated_block() {
    // The region is genuinely replaced by unrelated code. Fuzzy must NOT
    // claim a spurious match — invariant #4 prefers an honest orphan over a
    // confidently-wrong anchor.
    let v1 = file(
        "a.rs",
        &[
            "fn parse_header(buf: &[u8]) -> Header {",
            "    let magic = read_u32(buf);",
            "    Header { magic }",
            "}",
        ],
    );
    let bundle = SelectorBundle::capture(&v1, LineRange::new(1, 4).unwrap()).unwrap();

    let v2 = file(
        "a.rs",
        &[
            "const TIMEOUT: u64 = 30;",
            "static CACHE: Lazy<Map> = Lazy::new(Map::new);",
            "type Result<T> = std::result::Result<T, Error>;",
            "pub use crate::prelude::*;",
        ],
    );
    let res = resolve(&bundle, &v2, None);

    assert_eq!(res.range, None, "must orphan, not fabricate a match");
    assert!(matches!(res.status, AnchorStatus::Orphaned { .. }));
}

#[test]
fn fuzzy_relocates_a_single_line_after_a_rename() {
    let v1 = file(
        "x.rs",
        &[
            "fn a() {}",
            "    let total = compute_sum(items);",
            "fn b() {}",
        ],
    );
    let bundle = SelectorBundle::capture(&v1, LineRange::new(2, 2).unwrap()).unwrap();

    // Same line, one identifier renamed.
    let v2 = file(
        "x.rs",
        &[
            "fn a() {}",
            "    let total = compute_total(items);",
            "fn b() {}",
        ],
    );
    let res = resolve(&bundle, &v2, None);

    assert_eq!(res.range, Some(LineRange::new(2, 2).unwrap()));
    assert!(matches!(res.status, AnchorStatus::Drifted { .. }));
}

#[test]
fn fuzzy_relocates_a_region_with_every_local_renamed() {
    // Every local is renamed and a literal changed, but the structure is
    // preserved and the block has not moved. Exact equality (R2, and the old
    // exact-anchor-only R3) misses every line at once; the rename-tolerant
    // fuzzy rung must still recover it and report drifted, not orphaned.
    let v1 = file(
        "agg.rs",
        &[
            "fn aggregate(items: &[Item]) -> Summary {",
            "    let mut count = 0;",
            "    let mut total = 0;",
            "    for entry in items {",
            "        count += 1;",
            "        total += entry.amount;",
            "    }",
            "    Summary { count, total, limit: 100 }",
            "}",
        ],
    );
    let bundle = SelectorBundle::capture(&v1, LineRange::new(1, 9).unwrap()).unwrap();

    // `count` -> `n`, `total` -> `sum`, `entry` -> `row`, literal 100 -> 250.
    // Same line count, same position.
    let v2 = file(
        "agg.rs",
        &[
            "fn aggregate(items: &[Item]) -> Summary {",
            "    let mut n = 0;",
            "    let mut sum = 0;",
            "    for row in items {",
            "        n += 1;",
            "        sum += row.amount;",
            "    }",
            "    Summary { n, sum, limit: 250 }",
            "}",
        ],
    );
    let res = resolve(&bundle, &v2, None);

    assert!(
        matches!(res.status, AnchorStatus::Drifted { .. }),
        "an edited-in-place region must be drifted, not orphaned: {:?}",
        res.status
    );
    let r = res.range.expect("renamed region relocated, not orphaned");
    assert!(
        r.start() <= 1 && r.end() >= 9,
        "expected to cover the renamed block, got {}:{}",
        r.start(),
        r.end()
    );
}

#[test]
fn fuzzy_does_not_false_positive_on_an_unrelated_block_at_position() {
    // The position-anchored fallback must not turn a genuine replacement into
    // a spurious match: an unrelated block at the saved position still orphans.
    let v1 = file(
        "p.rs",
        &[
            "fn encode(value: u64) -> Vec<u8> {",
            "    let mut out = Vec::new();",
            "    push_varint(&mut out, value);",
            "    out",
            "}",
        ],
    );
    let bundle = SelectorBundle::capture(&v1, LineRange::new(1, 5).unwrap()).unwrap();

    let v2 = file(
        "p.rs",
        &[
            "struct Config {",
            "    retries: u8,",
            "    deadline: Duration,",
            "    verbose: bool,",
            "}",
        ],
    );
    let res = resolve(&bundle, &v2, None);

    assert_eq!(res.range, None, "must orphan, not fabricate a match");
    assert!(matches!(res.status, AnchorStatus::Orphaned { .. }));
}

#[test]
fn file_scoped_notes_match_any_query_on_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::init(dir.path()).unwrap();
    let target = dir.path().join("m.rs");
    std::fs::write(&target, "fn a() {}\nfn b() {}\n").unwrap();
    let src = SourceFile::read(&target).unwrap();
    let bundle = SelectorBundle::capture(&src, LineRange::new(1, 2).unwrap()).unwrap();
    let note = Note::new(
        store.relativize(&target).unwrap(),
        Scope::File,
        bundle,
        "module-wide context".to_owned(),
    )
    .unwrap();
    store.save(&note).unwrap();

    let result = query(&store, &target, LineSpec::Line(999)).unwrap();
    assert_eq!(result.matched.len(), 1);
    assert_eq!(result.matched[0].note.scope, Scope::File);
}

/// A note saved for `generateAccessToken` whose function was deleted must not
/// re-land on the surviving same-shaped sibling `generateRefreshToken`. The
/// two share scaffolding lines (`function NAME(user) {`, `const payload = …`,
/// `return signJwt(…);`, `}`), so fuzzy's LCS will hit the sibling's body.
/// The realistic surrounding context (imports above, the sibling well below
/// the original) puts the sibling's range far enough from the saved range
/// that the position rung does not overlap fuzzy's hit either — so no rung
/// independently corroborates fuzzy. The verdict must be `orphaned`, not a
/// confidently-wrong `drifted` that `reanchor` would then weld in place
/// (invariant #4).
#[test]
fn fuzzy_alone_landing_on_a_sibling_orphans_rather_than_misanchors() {
    let v1 = file(
        "auth.js",
        &[
            "import jwt from 'jsonwebtoken';",
            "function generateAccessToken(user) {",
            "  const payload = { sub: user.id, kind: 'access' };",
            "  const exp = Math.floor(Date.now() / 1000) + 900;",
            "  return signJwt({ ...payload, exp });",
            "}",
            "",
            "function generateRefreshToken(user) {",
            "  const payload = { sub: user.id, kind: 'refresh' };",
            "  const exp = Math.floor(Date.now() / 1000) + 86400 * 30;",
            "  return signJwt({ ...payload, exp });",
            "}",
        ],
    );
    let bundle = SelectorBundle::capture(&v1, LineRange::new(2, 6).unwrap()).unwrap();

    // generateAccessToken is gone; padding above keeps the sibling at lines
    // 8:12, well below the saved 2:6 — so the position rung (range 2:6,
    // clamped within the file) cannot overlap fuzzy's 8:12 hit.
    let v2 = file(
        "auth.js",
        &[
            "import jwt from 'jsonwebtoken';",
            "import { config } from './config.js';",
            "import { logger } from './logger.js';",
            "",
            "// auth.js — token helpers",
            "// (generateAccessToken removed; use OAuth provider instead)",
            "",
            "function generateRefreshToken(user) {",
            "  const payload = { sub: user.id, kind: 'refresh' };",
            "  const exp = Math.floor(Date.now() / 1000) + 86400 * 30;",
            "  return signJwt({ ...payload, exp });",
            "}",
        ],
    );
    let res = resolve(&bundle, &v2, None);

    assert_eq!(
        res.range, None,
        "fuzzy alone, with no overlap from any other rung, must orphan — \
         not silently re-anchor onto a same-shaped sibling"
    );
    assert!(matches!(res.status, AnchorStatus::Orphaned { .. }));
}

/// The more realistic workflow variant: the user saves a note for
/// `generateAccessToken` *after* the surrounding imports already exist (so
/// the saved bundle's position rung points at the function's current
/// location), then someone deletes that function much later. The sibling
/// `generateRefreshToken` sits two lines below the deletion. Position is now
/// only off by two — close enough that an `overlap`-only check would
/// trivially "corroborate" fuzzy's wrong hit. The corroboration rule
/// therefore treats position as a real witness only when fuzzy actually
/// stayed at the saved range; here fuzzy moved, no other rung supports it,
/// so the verdict must orphan.
#[test]
fn fuzzy_sibling_attack_is_caught_when_position_is_merely_nearby() {
    let common = [
        "import jwt from 'jsonwebtoken';",
        "import { config } from './config.js';",
        "import { logger } from './logger.js';",
        "",
        "// auth.js — token helpers",
    ];
    let v1: Vec<&str> = common
        .iter()
        .copied()
        .chain([
            "function generateAccessToken(user) {",
            "  const payload = { sub: user.id, kind: 'access' };",
            "  const exp = Math.floor(Date.now() / 1000) + 900;",
            "  return signJwt({ ...payload, exp });",
            "}",
            "",
            "function generateRefreshToken(user) {",
            "  const payload = { sub: user.id, kind: 'refresh' };",
            "  const exp = Math.floor(Date.now() / 1000) + 86400 * 30;",
            "  return signJwt({ ...payload, exp });",
            "}",
        ])
        .collect();
    let v1 = file("auth.js", &v1);
    let bundle = SelectorBundle::capture(&v1, LineRange::new(6, 10).unwrap()).unwrap();

    // Same file with generateAccessToken excised (sibling moves up to 8:12).
    let v2: Vec<&str> = common
        .iter()
        .copied()
        .chain([
            "// (generateAccessToken removed; use OAuth provider instead)",
            "",
            "function generateRefreshToken(user) {",
            "  const payload = { sub: user.id, kind: 'refresh' };",
            "  const exp = Math.floor(Date.now() / 1000) + 86400 * 30;",
            "  return signJwt({ ...payload, exp });",
            "}",
        ])
        .collect();
    let v2 = file("auth.js", &v2);
    let res = resolve(&bundle, &v2, None);

    assert_eq!(
        res.range, None,
        "position is the saved range; if fuzzy moved by 2 lines onto a \
         same-shaped sibling, position 'overlapping' fuzzy is not real evidence"
    );
    assert!(matches!(res.status, AnchorStatus::Orphaned { .. }));
}
