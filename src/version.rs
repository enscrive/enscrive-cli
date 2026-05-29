//! The canonical Enscrive `--version` string for this binary.
//!
//! Implements the cross-fleet `--version` standard (ENS-544) for `enscrive-cli`
//! via the clap-attribute path (ENS-553): clap's `version` attribute on the
//! top-level command is pointed at [`VERSION_LINE`], so `enscrive --version`
//! and `enscrive -V` both print exactly this string and exit cleanly.
//!
//! The git SHA and build date are injected at COMPILE time by `build.rs`
//! (`ENSCRIVE_GIT_SHA` / `ENSCRIVE_BUILD_DATE`) — there is no runtime git call
//! anywhere. Any other place that needs to report this binary's version MUST
//! use this same constant so the values can never disagree.

/// The full standard version line, e.g. `0.1.0+a3f9c1e (2026-05-28)`.
///
/// Clap prefixes the command name (`enscrive`) when printing `--version`, so
/// the line the user sees is `enscrive 0.1.0+a3f9c1e (2026-05-28)`, matching
/// the ENS-544 contract `<binary-name> <semver>+<git-sha> (<build-date>)`.
pub const VERSION_LINE: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "+",
    env!("ENSCRIVE_GIT_SHA"),
    " (",
    env!("ENSCRIVE_BUILD_DATE"),
    ")"
);

#[cfg(test)]
mod tests {
    use super::*;

    /// The line must be exactly `<semver>+<sha> (<YYYY-MM-DD>)`.
    #[test]
    fn version_line_matches_standard_shape() {
        // Split off the parenthesized build date.
        let (head, date_part) = VERSION_LINE
            .split_once(" (")
            .expect("version line must contain ' (' before the build date");
        let date = date_part
            .strip_suffix(')')
            .expect("version line must end with ')'");

        // head = "<semver>+<sha>"
        let (semver, sha) = head
            .split_once('+')
            .expect("version line must contain '+' between semver and sha");

        // semver is exactly CARGO_PKG_VERSION.
        assert_eq!(semver, env!("CARGO_PKG_VERSION"));

        // sha is non-empty; a real CI build is 7 hex chars, optionally with a
        // `-dirty` suffix. We don't hard-require 7 here because local/source
        // builds may legitimately stamp `unknown`.
        assert!(!sha.is_empty(), "git sha segment must not be empty");

        // build date is YYYY-MM-DD.
        let ymd: Vec<&str> = date.split('-').collect();
        assert_eq!(ymd.len(), 3, "build date must be YYYY-MM-DD, got '{date}'");
        assert_eq!(ymd[0].len(), 4, "year must be 4 digits, got '{}'", ymd[0]);
        assert_eq!(ymd[1].len(), 2, "month must be 2 digits, got '{}'", ymd[1]);
        assert_eq!(ymd[2].len(), 2, "day must be 2 digits, got '{}'", ymd[2]);
        for part in ymd {
            assert!(
                part.chars().all(|c| c.is_ascii_digit()),
                "build date component '{part}' must be all digits"
            );
        }
    }
}
