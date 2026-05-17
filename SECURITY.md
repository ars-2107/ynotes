# Security Policy

## Supported versions

ynotes is pre-1.0. Only the latest `main` and the most recent tagged release
receive security fixes until a stable line is established.

## Reporting a vulnerability

**Do not open a public issue for security reports.**

Use GitHub's private vulnerability reporting ("Report a vulnerability" under the
repository's *Security* tab), which opens a private advisory with the
maintainers.

Please include: affected version or commit, a description of the issue, and a
minimal reproduction if possible.

## What to expect

- Acknowledgement of the report within a few working days.
- An assessment and, where accepted, a fix tracked privately until a
  coordinated release.
- Credit in the release notes, unless you ask to remain anonymous.

## Scope

This is a local-first developer tool. The areas of most interest are: handling
of repository paths and the on-disk store (path traversal, symlink following),
processing of untrusted file content during anchoring, and the supply chain
(dependencies are gated in CI by `cargo-deny`).
