//! Serve embedded usage guides without accessing Git or a note store.
//! Only usage errors and stdout I/O can fail.

use std::fmt::Write as _;
use std::io::{self, Write};

use crate::cli::SkillsAction;
use crate::command_error::CommandError;

/// One served skill: its lookup name, its markdown, and the optional deep
/// appendix `--full` appends.
struct Skill {
    name: &'static str,
    content: &'static str,
    /// The `--full` appendix, a headed fragment printed after the main
    /// content. `None` for skills without a deep variant.
    full_extra: Option<&'static str>,
}

/// Every skill this binary serves, in the order `list` prints them.
const SKILLS: &[Skill] = &[
    Skill {
        name: "core",
        content: include_str!("../../skills/core.md"),
        full_extra: Some(include_str!("../../skills/core-reference.md")),
    },
    Skill {
        name: "sharing",
        content: include_str!("../../skills/sharing.md"),
        full_extra: None,
    },
    Skill {
        name: "maintenance",
        content: include_str!("../../skills/maintenance.md"),
        full_extra: None,
    },
];

/// The value of a `key:` line in the skill's frontmatter, trimmed.
///
/// A line scan, not a YAML parser: the frontmatter is ours and its values
/// are single-line by construction, so a YAML dependency would buy nothing.
fn frontmatter_value<'a>(content: &'a str, key: &str) -> Option<&'a str> {
    let mut lines = content.lines();
    if lines.next()? != "---" {
        return None;
    }
    let mut value = None;
    for line in lines {
        if line == "---" {
            return value;
        }
        if value.is_none() {
            value = line
                .strip_prefix(key)
                .and_then(|line| line.strip_prefix(':'))
                .map(str::trim)
                .filter(|value| !value.is_empty());
        }
    }
    None
}

/// Git may check embedded Markdown out with CRLF on Windows. Keep the CLI's
/// agent-facing output byte-stable across platforms.
fn normalize_newlines(content: &str) -> String {
    content.replace("\r\n", "\n")
}

/// The `list` output: two-space indent, name column padded to the longest
/// name, two spaces, the frontmatter description.
fn listing() -> String {
    let width = SKILLS.iter().map(|s| s.name.len()).max().unwrap_or(0);
    let mut out = String::new();
    for skill in SKILLS {
        let name = skill.name;
        let desc = frontmatter_value(skill.content, "description").unwrap_or_default();
        // Writing to a `String` is infallible; the codebase's render idiom.
        let _ = writeln!(out, "  {name:<width$}  {desc}");
    }
    out
}

/// The exact bytes `get` writes to stdout, or the usage error refusing the
/// request. Validation precedes rendering, so a refused `--full` prints
/// nothing, no silent fallback to the plain content.
fn rendered(name: &str, full: bool) -> Result<String, CommandError> {
    let Some(skill) = SKILLS.iter().find(|s| s.name == name) else {
        let names: Vec<&str> = SKILLS.iter().map(|s| s.name).collect();
        return Err(CommandError::Usage(format!(
            "no skill named `{name}`; available: {}",
            names.join(", ")
        )));
    };
    match (full, skill.full_extra) {
        (false, _) => Ok(normalize_newlines(skill.content)),
        // The extra newline leaves one blank line between the two files, so
        // the appendix's first `##` heading stays a heading when the output
        // is read as one document.
        (true, Some(extra)) => Ok(format!(
            "{}\n{}",
            normalize_newlines(skill.content),
            normalize_newlines(extra)
        )),
        (true, None) => Err(CommandError::Usage(format!(
            "`{name}` has no full variant; only `core` does. Run `ynotes skills get core --full`."
        ))),
    }
}

/// Serve the requested `skills` verb.
///
/// # Errors
///
/// [`CommandError::Usage`] for an unknown skill name or `--full` on a skill
/// without a full variant; [`CommandError::Io`] if stdout cannot be written
/// (e.g. a closed pipe). No engine error is reachable, the content is
/// embedded and nothing touches a store or git.
pub(crate) fn run(action: &SkillsAction) -> Result<(), CommandError> {
    let out = match action {
        SkillsAction::List => listing(),
        SkillsAction::Get { name, full } => rendered(name, *full)?,
    };
    let mut stdout = io::stdout().lock();
    stdout.write_all(out.as_bytes())?;
    stdout.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_skill_serves_nonempty_content_with_matching_frontmatter() {
        for skill in SKILLS {
            assert!(
                !skill.content.trim().is_empty(),
                "`{}` is empty",
                skill.name
            );
            assert_eq!(
                frontmatter_value(skill.content, "name"),
                Some(skill.name),
                "`{}`: frontmatter name must match the table key",
                skill.name
            );
            let desc = frontmatter_value(skill.content, "description");
            assert!(
                desc.is_some_and(|d| !d.is_empty()),
                "`{}` needs a one-line description",
                skill.name
            );
        }
    }

    #[test]
    fn frontmatter_is_read_from_crlf_source() {
        let content = "---\r\nname: core\r\ndescription: Test skill.\r\n---\r\n# Core\r\n";
        assert_eq!(frontmatter_value(content, "name"), Some("core"));
        assert_eq!(
            frontmatter_value(content, "description"),
            Some("Test skill.")
        );
    }

    #[test]
    fn unterminated_frontmatter_is_rejected() {
        assert_eq!(frontmatter_value("---\nname: core\n", "name"), None);
    }

    #[test]
    fn embedded_markdown_is_rendered_with_stable_line_endings() {
        assert_eq!(normalize_newlines("one\r\ntwo\r\n"), "one\ntwo\n");
    }

    /// Growth must be deliberate, the same spirit as the MCP instructions'
    /// 2KB client cap. 500 lines per content file is the authoring ceiling.
    #[test]
    fn every_content_file_stays_under_the_500_line_ceiling() {
        for skill in SKILLS {
            assert!(
                skill.content.lines().count() <= 500,
                "`{}` grew past 500 lines",
                skill.name
            );
            if let Some(extra) = skill.full_extra {
                assert!(
                    extra.lines().count() <= 500,
                    "`{}` appendix grew past 500 lines",
                    skill.name
                );
            }
        }
    }

    #[test]
    fn the_full_appendix_is_a_headed_fragment_without_frontmatter() {
        for skill in SKILLS {
            if let Some(extra) = skill.full_extra {
                assert!(
                    extra.starts_with("## "),
                    "`{}` appendix must start at a `##` heading",
                    skill.name
                );
            }
        }
    }

    #[test]
    fn core_ends_with_the_where_to_go_next_section() {
        let core = SKILLS
            .iter()
            .find(|s| s.name == "core")
            .expect("core is served");
        let last = core.content.lines().rev().find(|l| l.starts_with("## "));
        assert_eq!(last, Some("## Where to go next"));
    }

    #[test]
    fn listing_pads_names_and_carries_each_description() {
        let listing = listing();
        for skill in SKILLS {
            let desc = frontmatter_value(skill.content, "description").expect("pinned above");
            let line = listing
                .lines()
                .find(|l| l.trim_start().starts_with(skill.name))
                .expect("every skill is listed");
            assert!(line.starts_with("  "), "two-space indent");
            assert!(line.ends_with(desc), "description column intact");
        }
    }

    #[test]
    fn unknown_name_is_a_usage_error_listing_the_valid_names() {
        let err = rendered("everything", false).expect_err("unknown name must refuse");
        assert!(matches!(err, CommandError::Usage(_)));
        assert!(
            err.to_string()
                .contains("available: core, sharing, maintenance")
        );
    }

    #[test]
    fn full_appends_the_reference_after_one_blank_line() {
        let plain = rendered("core", false).expect("core serves");
        let full = rendered("core", true).expect("core has a full variant");
        assert!(
            full.starts_with(&plain),
            "--full appends; it never rewrites"
        );
        let appendix = &full[plain.len()..];
        assert!(
            appendix.starts_with("\n## "),
            "exactly one blank line, then the appendix"
        );
    }

    #[test]
    fn full_on_a_skill_without_a_full_variant_is_refused() {
        let err = rendered("sharing", true).expect_err("no silent fallback to plain content");
        assert!(matches!(err, CommandError::Usage(_)));
        assert!(err.to_string().contains("only `core`"));
    }
}
