//! `--help` carries no internal programme vocabulary (ENS-3237).
//!
//! Help text is the public face of the CLI: it is what a stranger reads
//! first, and what an agent scrapes to learn the surface. Ticket ids,
//! internal wave/unit labels, and references to documents nobody outside
//! the company can open are noise at best — at worst they read as an
//! unfinished tool that leaked its issue tracker.
//!
//! This walks the WHOLE command tree from the real binary rather than
//! grepping source, because what matters is the bytes a user receives:
//! a doc comment on a clap item ships, an identical one on a helper
//! function does not, and only running `--help` can tell them apart.
//! Internal references in ordinary `//` comments and in doc comments on
//! non-clap items are untouched by this test and are welcome to stay —
//! engineering context belongs in the code.
//!
//! Extends the per-command precedent in `revisions_cli.rs`
//! (`revisions_help_text_carries_no_banned_vocabulary`) to every command.

use std::collections::BTreeSet;
use std::process::Command;

/// Patterns that must never reach a user. Each is (regex-free) matched by
/// a small hand-rolled scanner so the test needs no regex dependency.
#[derive(Debug)]
enum Banned {
    /// A ticket prefix followed by `-` and digits, e.g. `ENS-3237`.
    TicketId(&'static str),
    /// A literal substring, e.g. `DESIGN §`.
    Literal(&'static str),
    /// A programme label: word followed by a single capital letter or
    /// digit token, e.g. `Wave A`, `Unit 3`, `Pillar 2`.
    Label(&'static str),
}

const BANNED: &[Banned] = &[
    Banned::TicketId("ENS"),
    Banned::TicketId("EV"),
    Banned::TicketId("J"),
    Banned::TicketId("WS"),
    Banned::TicketId("CLI-TIER"),
    Banned::Literal("DESIGN §"),
    Banned::Literal("ADR ENSCRIVE-"),
    Banned::Label("Wave"),
    Banned::Label("Unit"),
    Banned::Label("Pillar"),
];

/// `PREFIX-<digits>` anywhere in `text`.
fn has_ticket_id(text: &str, prefix: &str) -> bool {
    let needle = format!("{prefix}-");
    let bytes = text.as_bytes();
    for (idx, _) in text.match_indices(&needle) {
        // Must not be preceded by an identifier char, so `ENS` does not
        // match inside `LICENSE-` and `J` does not match inside `PROJ-`.
        if idx > 0 {
            let prev = bytes[idx - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'-' {
                continue;
            }
        }
        if text[idx + needle.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
        {
            return true;
        }
    }
    false
}

/// `Label ` followed by a single capital letter or a digit — `Wave A`,
/// `Unit 3`. Deliberately narrow: the plain words are fine on their own
/// ("a unit of work"), it is the enumerated programme label that leaks.
fn has_label(text: &str, label: &str) -> bool {
    let needle = format!("{label} ");
    for (idx, _) in text.match_indices(&needle) {
        let rest = &text[idx + needle.len()..];
        let mut chars = rest.chars();
        let Some(first) = chars.next() else { continue };
        let next = chars.next();
        let is_enumerator = (first.is_ascii_uppercase() || first.is_ascii_digit())
            && next.is_none_or(|c| !c.is_alphabetic());
        if is_enumerator {
            return true;
        }
    }
    false
}

fn violations(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for banned in BANNED {
        let hit = match banned {
            Banned::TicketId(prefix) => has_ticket_id(text, prefix),
            Banned::Literal(literal) => text.contains(literal),
            Banned::Label(label) => has_label(text, label),
        };
        if hit {
            found.push(format!("{banned:?}"));
        }
    }
    found
}

fn help_for(path: &[String]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_enscrive"))
        .args(path)
        .arg("--help")
        .output()
        .expect("spawn enscrive binary");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Subcommand names listed under the `Commands:` block of a help page.
fn subcommands(help: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_block = false;
    for line in help.lines() {
        if line.starts_with("Commands:") {
            in_block = true;
            continue;
        }
        if in_block {
            // Any other top-level section header ends the block.
            if line.ends_with(':') && !line.starts_with(' ') {
                break;
            }
            let trimmed = line.trim_start();
            let indent = line.len() - trimmed.len();
            // Command rows are indented two spaces; continuation lines of a
            // long description are indented further.
            if indent == 2
                && let Some(name) = trimmed.split_whitespace().next()
                && name != "help"
                && name.chars().all(|c| c.is_ascii_lowercase() || c == '-')
            {
                names.push(name.to_string());
            }
        }
    }
    names
}

#[test]
fn help_text_carries_no_internal_references() {
    let mut queue: Vec<Vec<String>> = vec![vec![]];
    let mut visited: BTreeSet<Vec<String>> = BTreeSet::new();
    let mut failures: Vec<String> = Vec::new();
    let mut pages = 0usize;

    while let Some(path) = queue.pop() {
        if !visited.insert(path.clone()) {
            continue;
        }
        let help = help_for(&path);
        pages += 1;

        let found = violations(&help);
        if !found.is_empty() {
            failures.push(format!(
                "  `enscrive {} --help` leaks {}",
                path.join(" "),
                found.join(", ")
            ));
        }

        for sub in subcommands(&help) {
            let mut child = path.clone();
            child.push(sub);
            queue.push(child);
        }
    }

    assert!(
        pages > 100,
        "expected to walk the whole command tree; only reached {pages} help pages \
         — the subcommand scraper is probably broken, which would make this test \
         pass vacuously"
    );

    assert!(
        failures.is_empty(),
        "internal references reached --help on {} of {pages} pages:\n{}\n\n\
         Fix the doc comment / #[arg(help=..)] at source. Keep the prose \
         grammatical after removal — no dangling parentheticals.",
        failures.len(),
        failures.join("\n")
    );
}

/// The scanner has to actually catch things. A hygiene test that silently
/// stopped matching would be worse than no test at all.
#[test]
fn scanner_catches_what_it_claims_to() {
    assert!(!violations("ENS-3237 filter by job_kind").is_empty());
    assert!(!violations("Wave A: return immediately").is_empty());
    assert!(!violations("cleaning up staging corpora (J-024 Unit 3)").is_empty());
    assert!(!violations("Rate-limit governor commands (DESIGN §R9)").is_empty());
    assert!(!violations("ADR ENSCRIVE-CLI-APP-MEMORY-2026-07-31").is_empty());
    assert!(!violations("Metering backfill (Pillar 2 M3.2-5)").is_empty());

    // ...and must not fire on legitimate help prose.
    assert!(violations("Return immediately with the launched job").is_empty());
    assert!(violations("Filter by parent job UUID").is_empty());
    assert!(violations("Amount to credit, in MICROS (1,000,000 micros = $1.00)").is_empty());
    assert!(violations("RFC-3339 lower bound on created_at").is_empty());
    assert!(violations("the unit of work is a chunk").is_empty());
    assert!(violations("Promote a corpus into another environment").is_empty());
}
