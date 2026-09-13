//! Syntax-tree selectors for named constructs and their content fingerprints.
//!
//! Matching uses the ancestor path, node kinds, identifiers, and literals.
//! This tolerates formatting changes while distinguishing same-named constructs
//! in different scopes. Unsupported languages contribute no structural evidence.
//! Tree-sitter's grammar crates own their FFI; this crate contains no unsafe code.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::selector::sha256_hex;
use crate::source::LineRange;

/// Visited-node cap when fingerprinting a subtree, defends against a
/// pathologically large construct without affecting realistic code.
const MAX_FINGERPRINT_NODES: usize = 8000;

/// A source language ynotes can parse for structural anchoring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Language {
    /// Rust.
    Rust,
    /// JavaScript.
    JavaScript,
    /// TypeScript (non-JSX).
    TypeScript,
    /// TypeScript with JSX.
    Tsx,
    /// Python.
    Python,
    /// Go.
    Go,
    /// Java.
    Java,
    /// C.
    C,
    /// C++.
    Cpp,
    /// Ruby.
    Ruby,
    /// Bash / shell.
    Bash,
    /// JSON.
    Json,
    /// HTML.
    Html,
    /// CSS.
    Css,
}

impl Language {
    /// Detects the language from a path's extension, or `None` if it is not
    /// one ynotes parses (R4 then simply does not contribute).
    #[must_use]
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "rs" => Self::Rust,
            "js" | "mjs" | "cjs" | "jsx" => Self::JavaScript,
            "ts" | "mts" | "cts" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "py" | "pyi" => Self::Python,
            "go" => Self::Go,
            "java" => Self::Java,
            "c" | "h" => Self::C,
            "cc" | "cpp" | "cxx" | "hpp" | "hh" => Self::Cpp,
            "rb" => Self::Ruby,
            "sh" | "bash" => Self::Bash,
            "json" => Self::Json,
            "html" | "htm" => Self::Html,
            "css" => Self::Css,
            _ => return None,
        })
    }

    /// The tree-sitter grammar for this language.
    fn grammar(self) -> tree_sitter::Language {
        match self {
            Self::Rust => tree_sitter_rust::LANGUAGE.into(),
            Self::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Self::Python => tree_sitter_python::LANGUAGE.into(),
            Self::Go => tree_sitter_go::LANGUAGE.into(),
            Self::Java => tree_sitter_java::LANGUAGE.into(),
            Self::C => tree_sitter_c::LANGUAGE.into(),
            Self::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            Self::Ruby => tree_sitter_ruby::LANGUAGE.into(),
            Self::Bash => tree_sitter_bash::LANGUAGE.into(),
            Self::Json => tree_sitter_json::LANGUAGE.into(),
            Self::Html => tree_sitter_html::LANGUAGE.into(),
            Self::Css => tree_sitter_css::LANGUAGE.into(),
        }
    }
}

/// The R4 selector stored in a note's bundle: enough to re-find the region
/// by syntax after the text is reformatted or the construct is moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StructuralSelector {
    /// The language the region was parsed as.
    pub language: Language,
    /// Kind of the nearest named construct enclosing the region.
    pub node_kind: String,
    /// Named-ancestor chain, root → that construct (inclusive).
    pub path: Vec<PathStep>,
    /// Content fingerprint of that construct's subtree: node kinds plus the
    /// text of every identifier and literal, so a rename or a changed value
    /// is visible while a pure reformat is not.
    pub fingerprint: String,
    /// Region start as a line offset within the enclosing construct.
    pub rel_start: u32,
    /// Region end as a line offset within the enclosing construct.
    pub rel_end: u32,
}

/// One step on the root→region path: a named construct's kind and its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathStep {
    /// The node kind (e.g. `function_item`, `class_declaration`).
    pub kind: String,
    /// The construct's name, if it has one (functions, classes, methods…).
    pub name: Option<String>,
}

/// Parses `text` and captures a structural selector for `region`, or `None`
/// if it does not parse or has no named construct enclosing the region.
#[must_use]
pub fn capture(text: &str, language: Language, region: LineRange) -> Option<StructuralSelector> {
    let tree = parse(text, language)?;
    let src = text.as_bytes();
    let (s0, e0) = (region.start() - 1, region.end() - 1); // 0-based rows

    // For the AST search only, trim whitespace-only rows off both ends of the
    // region: a natural human selection includes the blank line after a
    // function ("the whole block"), and that blank is a sibling of the
    // function in the tree-sitter parse, so the smallest *named* node
    // covering the original region is the unnamed root, and structural
    // captures nothing. Trimming the blanks lets the search find the
    // function itself; `rel_start`/`rel_end` stay derived from the original
    // region so the user's selection, including the trailing blank, is
    // reproduced when the construct moves.
    let (s_search, e_search) = trim_blank_rows(text, s0, e0).unwrap_or((s0, e0));
    let anchor = smallest_named_containing(tree.root_node(), s_search, e_search, src)?;
    let path = named_chain(anchor, src);
    let anchor_row = u32::try_from(anchor.start_position().row).ok()?;

    Some(StructuralSelector {
        language,
        node_kind: anchor.kind().to_owned(),
        path,
        fingerprint: fingerprint(anchor, src),
        rel_start: s0.saturating_sub(anchor_row),
        rel_end: e0.saturating_sub(anchor_row),
    })
}

/// Shrinks `[s, e]` (0-based row range) inward past leading/trailing rows
/// that are blank or contain only whitespace. Returns `None` if every row in
/// the range is blank (no anchor to find).
fn trim_blank_rows(text: &str, s: u32, e: u32) -> Option<(u32, u32)> {
    let lines: Vec<&str> = text.split('\n').collect();
    let end_exclusive = (e as usize).saturating_add(1).min(lines.len());
    let start = s as usize;
    if start >= end_exclusive {
        return None;
    }
    let is_blank = |row: usize| lines.get(row).is_none_or(|l| l.trim().is_empty());
    let mut lo = start;
    let mut hi = end_exclusive - 1;
    while lo <= hi && is_blank(lo) {
        lo += 1;
    }
    while hi > lo && is_blank(hi) {
        hi -= 1;
    }
    if lo > hi || is_blank(lo) {
        return None;
    }
    Some((u32::try_from(lo).ok()?, u32::try_from(hi).ok()?))
}

/// Relocates the region described by `sel` in `text`, returning the new range
/// and a 0..=1 confidence (1.0 = same construct, identical structure, a pure
/// reformat or move; lower = the construct's body changed, or only a relaxed
/// name match was possible). `None` if R4 cannot contribute.
#[must_use]
pub(crate) fn locate(
    sel: &StructuralSelector,
    text: &str,
    language: Language,
) -> Option<(LineRange, f64)> {
    let tree = parse(text, language)?;
    let src = text.as_bytes();
    let last = sel.path.last()?;

    // All named nodes whose (kind, name) match the final path step.
    let mut candidates: Vec<tree_sitter::Node> = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if node.kind() == last.kind && node_name(node, src) == last.name {
            candidates.push(node);
        }
        let mut cur = node.walk();
        for child in node.named_children(&mut cur) {
            stack.push(child);
        }
    }
    if candidates.is_empty() {
        return None;
    }

    // Best = exact ancestor chain first, then identical fingerprint, then
    // nearest to the original position.
    let mut best: Option<(f64, tree_sitter::Node)> = None;
    for cand in candidates {
        let exact = named_chain(cand, src) == sel.path;
        let same_fp = fingerprint(cand, src) == sel.fingerprint;
        let sim = match (exact, same_fp) {
            (true, true) => 1.0,
            (true, false) => 0.80,
            (false, true) => 0.70,
            (false, false) => 0.55,
        };
        if best.as_ref().is_none_or(|(b, _)| sim > *b) {
            best = Some((sim, cand));
        }
    }

    let (sim, node) = best?;
    let base = u32::try_from(node.start_position().row).ok()?;
    let total = u32::try_from(text.lines().count()).unwrap_or(u32::MAX);
    let start = (base + sel.rel_start + 1).min(total.max(1));
    let end = (base + sel.rel_end + 1).min(total.max(1)).max(start);
    Some((LineRange::new(start, end).ok()?, sim))
}

/// Parses `text`; `None` on grammar or parser failure (R4 stays silent).
fn parse(text: &str, language: Language) -> Option<tree_sitter::Tree> {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language.grammar()).ok()?;
    parser.parse(text, None)
}

/// A node is a path anchor if it names something, the cross-language test
/// (functions, classes, methods, structs… all expose a `name` field).
///
/// Rust `impl T` is the exception: tree-sitter-rust carries its identity in
/// the `type` and (for trait impls) `trait` fields, not `name`. Without this
/// fallback the impl ancestor disappears from a method's path, so two
/// same-named methods on different types, `impl A { fn foo() {} }` and
/// `impl B { fn foo() {} }`, share the path `[function_item: foo]` and the
/// rung can vote for the wrong target.
fn node_name(node: tree_sitter::Node, src: &[u8]) -> Option<String> {
    let field = |f: &str| {
        node.child_by_field_name(f)
            .and_then(|n| n.utf8_text(src).ok())
            .map(str::to_owned)
    };
    field("name").or_else(|| match node.kind() {
        "impl_item" => match (field("trait"), field("type")) {
            (Some(tr), Some(ty)) => Some(format!("{tr} for {ty}")),
            (None, Some(ty)) => Some(ty),
            _ => None,
        },
        _ => None,
    })
}

/// Whether a node qualifies as a path anchor (it has a name).
fn qualifies(node: tree_sitter::Node, src: &[u8]) -> bool {
    node_name(node, src).is_some()
}

/// Whether `node` spans every line of the 0-based row range `[s, e]`.
fn covers(node: tree_sitter::Node, s: u32, e: u32) -> bool {
    let (ns, ne) = (
        node.start_position().row as u64,
        node.end_position().row as u64,
    );
    ns <= u64::from(s) && u64::from(e) <= ne
}

/// Descends from `root` to the smallest *named* node still covering the
/// region.
fn smallest_named_containing<'t>(
    root: tree_sitter::Node<'t>,
    s: u32,
    e: u32,
    src: &[u8],
) -> Option<tree_sitter::Node<'t>> {
    let mut cur = root;
    let mut best = None;
    loop {
        if qualifies(cur, src) && covers(cur, s, e) {
            best = Some(cur);
        }
        let mut walk = cur.walk();
        let next = cur.named_children(&mut walk).find(|c| covers(*c, s, e));
        match next {
            Some(child) => cur = child,
            None => break,
        }
    }
    best
}

/// The chain of named ancestors from root down to `node`, inclusive.
fn named_chain(node: tree_sitter::Node, src: &[u8]) -> Vec<PathStep> {
    let mut steps = Vec::new();
    let mut cur = Some(node);
    while let Some(n) = cur {
        if qualifies(n, src) {
            steps.push(PathStep {
                kind: n.kind().to_owned(),
                name: node_name(n, src),
            });
        }
        cur = n.parent();
    }
    steps.reverse();
    steps
}

/// Content fingerprint of a subtree: a pre-order walk of its named nodes
/// (comments excluded), emitting each node's kind and, for every named
/// *leaf*, i.e. an identifier, literal, or primitive type name, its text.
///
/// Whitespace and comments never appear, so a pure reformat yields the same
/// fingerprint; a renamed identifier or a changed literal value does not.
/// That is what lets the structural rung tell an intact construct from an
/// edited one: an identical fingerprint means the code is unchanged bar
/// formatting, a different one means its contents moved on.
fn fingerprint(node: tree_sitter::Node, src: &[u8]) -> String {
    let mut acc = String::new();
    let mut stack = vec![node];
    let mut seen = 0usize;
    while let Some(n) = stack.pop() {
        seen += 1;
        if seen > MAX_FINGERPRINT_NODES {
            break;
        }
        // Comment nodes are skipped so a comment-only edit yields the same
        // fingerprint. The kind string is grammar-specific: 9 of the 13
        // grammars ynotes parses (JS, TS, Tsx, Python, Go, C, C++, Ruby,
        // Bash, HTML, CSS) emit the literal `"comment"`, while Rust and
        // Java emit `"line_comment"`/`"block_comment"`. Matching only
        // `"comment"` therefore silently retained Rust/Java comments in the
        // fingerprint, contradicting the rest of this docstring for the
        // project's own primary language.
        if matches!(n.kind(), "comment" | "line_comment" | "block_comment") {
            continue;
        }
        acc.push_str(n.kind());
        let mut walk = n.walk();
        let mut has_named_child = false;
        for child in n.named_children(&mut walk) {
            has_named_child = true;
            stack.push(child);
        }
        // A named leaf is a content atom, an identifier, a literal, a type
        // name. Bind its text in, so a rename or a changed value is detected;
        // an internal node contributes only its kind, keeping the fingerprint
        // stable across reformatting.
        if !has_named_child && let Ok(text) = n.utf8_text(src) {
            acc.push(':');
            acc.push_str(text);
        }
        acc.push('/');
    }
    sha256_hex(acc.as_bytes())
}
