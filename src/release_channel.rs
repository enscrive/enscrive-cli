//! Release channel: naming conventions + platform detection for binaries
//! fetched from the release manifest at enscrive.io/releases/.
//!
//! Target convention: Rust target triples (x86_64-unknown-linux-gnu, etc.).
//! The CLI reports its own compile-time target, not a runtime guess —
//! whatever libc/arch the CLI is linked against, service binaries with the
//! same target triple will run on the same host.

/// Compile-time target triple of this CLI binary.
/// Injected by build.rs from cargo's TARGET env var.
pub const CURRENT_TARGET: &str = env!("CLI_TARGET");

pub fn current_target() -> &'static str {
    CURRENT_TARGET
}

/// Format the "platform not in manifest" error message.
///
/// Lists the available platforms from the manifest (sorted) and points at
/// the build-from-source docs for unsupported hosts.
pub fn format_platform_missing(url: &str, platform: &str, available: &[String]) -> String {
    let mut sorted: Vec<&str> = available.iter().map(|s| s.as_str()).collect();
    sorted.sort();
    let list = sorted
        .iter()
        .map(|p| format!("  - {}", p))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "release manifest '{}' does not include platform '{}'.\n\
         Available platforms in this release:\n{}\n\
         If your platform isn't supported, build from source: https://enscrive.io/docs/building-from-source",
        url, platform, list
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_target_is_non_empty() {
        assert!(!current_target().is_empty());
        assert!(!CURRENT_TARGET.is_empty());
    }

    #[test]
    fn current_target_matches_expected_shape() {
        // All five approved targets contain one of these substrings.
        let t = current_target();
        let ok = t.contains("linux-gnu") || t.contains("linux-musl") || t.contains("apple-darwin");
        assert!(
            ok,
            "unexpected target triple shape: '{}' (expected to contain linux-gnu, linux-musl, or apple-darwin)",
            t
        );
    }

    #[test]
    fn format_platform_missing_sorts_available_and_includes_docs() {
        let available = vec![
            "x86_64-unknown-linux-musl".to_string(),
            "aarch64-apple-darwin".to_string(),
            "x86_64-unknown-linux-gnu".to_string(),
        ];
        let msg = format_platform_missing(
            "https://example.test/manifest.json",
            "riscv64-unknown-linux-gnu",
            &available,
        );

        // Contains the manifest URL and the missing platform.
        assert!(msg.contains("https://example.test/manifest.json"));
        assert!(msg.contains("riscv64-unknown-linux-gnu"));

        // All available entries appear.
        assert!(msg.contains("x86_64-unknown-linux-musl"));
        assert!(msg.contains("aarch64-apple-darwin"));
        assert!(msg.contains("x86_64-unknown-linux-gnu"));

        // Entries are sorted: aarch64-apple-darwin precedes x86_64-unknown-linux-gnu
        // precedes x86_64-unknown-linux-musl.
        let pos_aarch = msg.find("aarch64-apple-darwin").unwrap();
        let pos_gnu = msg.find("x86_64-unknown-linux-gnu").unwrap();
        let pos_musl = msg.find("x86_64-unknown-linux-musl").unwrap();
        assert!(pos_aarch < pos_gnu);
        assert!(pos_gnu < pos_musl);

        // Docs URL is present.
        assert!(msg.contains("https://enscrive.io/docs/building-from-source"));
    }
}
