//! Structural identity across formatting, moves, and deletion.
//! A surviving enclosing construct must not identify a deleted inner region.

use ynotes::{
    AnchorStatus, Language, LineRange, PathStep, SelectorBundle, SourceFile, resolve,
    structural_capture,
};

fn file(lines: &[&str]) -> SourceFile {
    SourceFile::from_lines(
        "m.rs".into(),
        lines.iter().map(|s| (*s).to_owned()).collect(),
    )
}

/// Builds a bundle for `range` of `v1`, including the structural selector.
fn bundle_for(v1: &SourceFile, range: LineRange) -> SelectorBundle {
    let mut b = SelectorBundle::capture(v1, range).unwrap();
    let sel = structural_capture(&v1.text_all(), Language::Rust, range).expect("rust parses");
    b = b.with_structural(sel);
    b
}

#[test]
fn structural_survives_reformat_and_move() {
    let v1 = file(&[
        "fn alpha() {",
        "    let x = 1;",
        "}",
        "fn target(a: i32) -> i32 {",
        "    let y = a + 1;",
        "    y",
        "}",
        "fn omega() {}",
    ]);
    // The note is on `fn target` (lines 4..=7).
    let bundle = bundle_for(&v1, LineRange::new(4, 7).unwrap());

    // Prepend comments AND reformat target with blank lines: the text rungs
    // lose it, structural must not.
    let v2 = file(&[
        "// added banner",
        "// second line",
        "fn alpha() {",
        "    let x = 1;",
        "}",
        "",
        "fn target(a: i32) -> i32 {",
        "",
        "    let y = a + 1;",
        "    y",
        "",
        "}",
        "fn omega() {}",
    ]);
    let res = resolve(&bundle, &v2, None);

    let r = res.range.expect("relocated, not orphaned");
    // Reformatted and moved, but the construct's fingerprint is identical:
    // the note is anchored at its new position, not stale.
    assert!(
        matches!(res.status, AnchorStatus::Anchored),
        "moved but intact ⇒ anchored, got {:?}",
        res.status
    );
    // `fn target` now begins at line 7 in v2.
    assert!(
        r.start() >= 7,
        "expected to track the moved construct, got {}:{}",
        r.start(),
        r.end()
    );
}

#[test]
fn indentation_only_change_stays_anchored() {
    let v1 = file(&["fn target() {", "    let y = 1;", "    y", "}"]);
    let bundle = bundle_for(&v1, LineRange::new(1, 4).unwrap());

    // Same lines/positions, body re-indented: exact-quote fails, structure is
    // identical and unmoved ⇒ still anchored.
    let v2 = file(&["fn target() {", "        let y = 1;", "        y", "}"]);
    let res = resolve(&bundle, &v2, None);

    assert_eq!(res.range, Some(LineRange::new(1, 4).unwrap()));
    assert!(
        matches!(res.status, AnchorStatus::Anchored),
        "indentation-only ⇒ anchored, got {:?}",
        res.status
    );
}

#[test]
fn a_deleted_construct_still_orphans() {
    let v1 = file(&[
        "fn keep() {}",
        "fn doomed(n: usize) -> usize {",
        "    n * 2",
        "}",
    ]);
    let bundle = bundle_for(&v1, LineRange::new(2, 4).unwrap());

    // `doomed` is gone and replaced with unrelated content.
    let v2 = file(&["fn keep() {}", "const X: usize = 1;", "type Y = u8;"]);
    let res = resolve(&bundle, &v2, None);

    assert_eq!(res.range, None);
    assert!(matches!(res.status, AnchorStatus::Orphaned { .. }));
}

#[test]
fn a_partial_deletion_inside_a_surviving_function_orphans() {
    // The note covers a sub-span *inside* `target`, three plain statements,
    // not a named construct of their own. Delete those lines: `target` still
    // exists, so the structural rung still recognises the *function*, but the
    // noted code is gone. quote and fuzzy miss; a structural hit on the
    // surviving container is not a relocation of the region, so the verdict
    // must be `orphaned`, never a confident false-anchor onto other code.
    let v1 = file(&[
        "fn keep() {}",
        "fn target() {",
        "    let a = 1;",
        "    let b = 2;",
        "    let c = 3;",
        "    done();",
        "}",
    ]);
    let bundle = bundle_for(&v1, LineRange::new(3, 5).unwrap());

    // Delete the three `let`s. `target` survives as `fn target() { done(); }`.
    let v2 = file(&["fn keep() {}", "fn target() {", "    done();", "}"]);
    let res = resolve(&bundle, &v2, None);

    assert!(
        matches!(res.status, AnchorStatus::Orphaned { .. }),
        "a deleted sub-span must orphan even though its function survives: {:?}",
        res.status
    );
    assert_eq!(res.range, None, "an orphan has no resolved range");
}

/// `impl T` is a path anchor: tree-sitter-rust binds its identity to the
/// `type` field rather than `name`, so the fallback in `node_name` is what
/// stops the impl ancestor from being dropped from a method's path.
#[test]
fn impl_block_is_a_named_path_anchor() {
    let v1 = file(&["impl Invoice {", "    pub fn total() {}", "}"]);
    // A note on the impl header line resolves to the impl block itself.
    let sel = structural_capture(
        &v1.text_all(),
        Language::Rust,
        LineRange::new(1, 1).unwrap(),
    )
    .expect("impl_item must qualify as a path anchor");
    assert_eq!(sel.node_kind, "impl_item");
    assert_eq!(
        sel.path,
        vec![PathStep {
            kind: "impl_item".into(),
            name: Some("Invoice".into()),
        }],
    );

    // A note inside the impl carries the impl as an ancestor.
    let inner = structural_capture(
        &v1.text_all(),
        Language::Rust,
        LineRange::new(2, 2).unwrap(),
    )
    .unwrap();
    assert_eq!(
        inner.path,
        vec![
            PathStep {
                kind: "impl_item".into(),
                name: Some("Invoice".into()),
            },
            PathStep {
                kind: "function_item".into(),
                name: Some("total".into()),
            },
        ],
    );
}

/// `impl Trait for Type` carries both. Combining them into one synthetic
/// name keeps `PathStep` flat (no schema change) while still letting
/// `locate()` reject candidates whose trait-or-type differs.
#[test]
fn trait_impl_path_step_combines_trait_and_type() {
    let v1 = file(&[
        "impl std::fmt::Display for Invoice {",
        "    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { Ok(()) }",
        "}",
    ]);
    let sel = structural_capture(
        &v1.text_all(),
        Language::Rust,
        LineRange::new(2, 2).unwrap(),
    )
    .expect("captured");
    assert_eq!(sel.path.len(), 2);
    assert_eq!(sel.path[0].kind, "impl_item");
    assert_eq!(
        sel.path[0].name.as_deref(),
        Some("std::fmt::Display for Invoice"),
    );
    assert_eq!(sel.path[1].name.as_deref(), Some("fmt"));
}

/// The collision the previous behaviour produced: a method noted in
/// `impl A` must not relocate to a same-named method in `impl B`. Before the
/// fix, both methods shared the path `[function_item: name]`, so the rung
/// voted for the wrong target; the resolver's dissent penalty hid the bug by
/// orphaning. With the impl ancestor present, `locate()` rejects the wrong
/// candidate up front.
#[test]
fn same_named_methods_on_different_impls_do_not_collide() {
    let v1 = file(&[
        "impl A {",
        "    pub fn total(&self) -> u32 {",
        "        self.seats() * self.rate()",
        "    }",
        "}",
        "impl B {",
        "    pub fn total(&self) -> &'static str { \"unrelated\" }",
        "}",
    ]);
    // Note on A::total's body.
    let bundle = bundle_for(&v1, LineRange::new(2, 4).unwrap());

    // Rename A::total to total_v2 (same body); B::total stays put. Quote
    // misses (signature changed), fuzzy can drift to A::total_v2's body,
    // structural must NOT cluster with B::total.
    let v2 = file(&[
        "impl A {",
        "    pub fn total_v2(&self) -> u32 {",
        "        self.seats() * self.rate()",
        "    }",
        "}",
        "impl B {",
        "    pub fn total(&self) -> &'static str { \"unrelated\" }",
        "}",
    ]);
    let res = resolve(&bundle, &v2, None);
    if let Some(r) = res.range {
        assert!(
            r.start() < 6,
            "must not resolve into impl B (lines 6+); got {}:{}",
            r.start(),
            r.end(),
        );
    }
}

#[test]
fn renamed_locals_are_not_anchored() {
    // Structure preserved, but every local is renamed and a literal changed.
    // A node-kind-only fingerprint cannot see that and would wrongly call the
    // region intact; the content fingerprint includes identifier and literal
    // text, so the edit registers and the region must not be `anchored`.
    let v1 = file(&[
        "fn target() {",
        "    let count = 0;",
        "    let total = count + 1;",
        "    total",
        "}",
    ]);
    let bundle = bundle_for(&v1, LineRange::new(1, 5).unwrap());

    let v2 = file(&[
        "fn target() {",
        "    let n = 0;",
        "    let sum = n + 2;",
        "    sum",
        "}",
    ]);
    let res = resolve(&bundle, &v2, None);

    assert!(
        !matches!(res.status, AnchorStatus::Anchored),
        "renamed locals changed the content, must not be `anchored`: {:?}",
        res.status
    );
}

/// A user range that includes the blank line *after* a function, a natural
/// "select the whole block" gesture, must still capture a structural anchor.
/// The trailing blank is a sibling of the function in the AST; without the
/// blank-row trim in `capture`, the smallest *named* node covering the
/// region is the unnamed root and structural silently drops the rung for
/// the note's entire lifetime.
#[test]
fn capture_handles_a_trailing_blank_row_in_the_region() {
    let v1 = file(&[
        "fn target(a: i32) -> i32 {",
        "    let y = a + 1;",
        "    y",
        "}",
        "",
        "fn next() {}",
    ]);
    // Lines 1..=5: the function plus the blank row after it.
    let sel = structural_capture(
        &v1.text_all(),
        Language::Rust,
        LineRange::new(1, 5).unwrap(),
    )
    .expect("trailing blank row must not skip structural");
    assert_eq!(sel.node_kind, "function_item");
    assert_eq!(
        sel.path.last().and_then(|s| s.name.as_deref()),
        Some("target"),
    );
}

/// And the captured selector still re-locates the region after a reformat-
/// plus-move: the trim affects only the AST search, not the `rel_start` /
/// `rel_end` the user actually selected.
#[test]
fn captured_with_trailing_blank_relocates_after_move() {
    let v1 = file(&[
        "fn target(a: i32) -> i32 {",
        "    let y = a + 1;",
        "    y",
        "}",
        "",
        "fn next() {}",
    ]);
    let bundle = bundle_for(&v1, LineRange::new(1, 5).unwrap());

    // Prepend two lines; the function and the blank after it shift down.
    let v2 = file(&[
        "// banner",
        "// second",
        "fn target(a: i32) -> i32 {",
        "    let y = a + 1;",
        "    y",
        "}",
        "",
        "fn next() {}",
    ]);
    let res = resolve(&bundle, &v2, None);
    let r = res.range.expect("relocated");
    assert!(
        r.start() >= 3 && r.end() >= 7,
        "expected to track the moved function + trailing blank, got {}:{}",
        r.start(),
        r.end(),
    );
}

/// A note's construct is deleted while a *same-named* construct in a different
/// scope survives elsewhere, `Foo::build` is removed, `Bar::build` remains.
/// Structural can only match same-named candidates, so it lands on `Bar::build`
/// but at a *relaxed, different-chain* score (0.55: same name, different
/// enclosing impl and body), and fuzzy latches onto its near-identical body at
/// the same place. Neither independently confirms `Foo::build` survived, they
/// converge on a same-named sibling. A relaxed structural hit must therefore
/// NOT witness fuzzy's move: the note orphans rather than welding onto
/// `Bar::build` (which `reanchor` would then make permanent). Same class as the
/// touched-git-witness hole, on the structural side.
#[test]
fn a_relaxed_structural_hit_does_not_witness_a_moved_fuzzy_onto_a_same_named_sibling() {
    let v1 = file(&[
        "impl Foo {",
        "    fn build(&self) -> Widget {",
        "        let mut w = Widget::new();",
        "        w.tune(self.alpha);",
        "        w",
        "    }",
        "}",
        "fn spacer_one() {}",
        "fn spacer_two() {}",
        "fn spacer_three() {}",
        "impl Bar {",
        "    fn build(&self) -> Widget {",
        "        let mut w = Widget::new();",
        "        w.tune(self.beta);",
        "        w",
        "    }",
        "}",
    ]);
    // Note on Foo::build's whole definition (lines 2:6).
    let bundle = bundle_for(&v1, LineRange::new(2, 6).unwrap());

    // Delete impl Foo entirely; impl Bar (with its own `build`) shifts up to the
    // top. `Foo::build` is genuinely gone, the note must not follow onto
    // `Bar::build`, a different method that merely shares the name.
    let v2 = file(&[
        "fn spacer_one() {}",
        "fn spacer_two() {}",
        "fn spacer_three() {}",
        "impl Bar {",
        "    fn build(&self) -> Widget {",
        "        let mut w = Widget::new();",
        "        w.tune(self.beta);",
        "        w",
        "    }",
        "}",
    ]);
    let res = resolve(&bundle, &v2, None);

    assert!(
        matches!(res.status, AnchorStatus::Orphaned { .. }),
        "Foo::build was deleted; a relaxed same-name structural hit on Bar::build \
         must not witness fuzzy's move, orphan, not drift: {:?} at {:?}",
        res.status,
        res.range,
    );
    assert_eq!(res.range, None, "an orphan has no resolved range");
}
