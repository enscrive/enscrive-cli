//! Release fetching is deliberately unsupported here; operator overrides remain available.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
type Result<T> = std::result::Result<T, String>;
fn fail(s: &str) -> String {
    format!("artifact trust: {s}")
}
fn valid_digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
#[path = "policy.rs"]
mod policy;
pub(crate) use policy::{Policy, Provenance};
fn unsupported() -> String {
    fail("release fetch currently supports Linux only; explicit operator binaries remain supported")
}
pub(crate) struct Aggregate;
pub(crate) struct Selection;
impl Aggregate {
    pub(crate) async fn load(_: &Policy, _: &Path, _: &str) -> Result<Self> {
        Err(unsupported())
    }
    pub(crate) fn select(&self, _: &str, _: &str) -> Result<Selection> {
        Err(unsupported())
    }
}
impl Selection {
    pub(crate) async fn install(&self, _: &Path, _: bool) -> Result<(String, Provenance)> {
        Err(unsupported())
    }
}
