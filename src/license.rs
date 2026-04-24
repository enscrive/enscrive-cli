//! License file helpers: path resolution, JWT decode (no-verify), write.
//!
//! Shared by `license activate`, `license status`, `license deactivate`.
//! ENS-63 / CLI-TIER-008.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

/// Resolve the on-disk license path.
///
/// Precedence:
///   1. `ENSCRIVE_LICENSE_PATH` env var, if set and non-empty.
///   2. `$HOME/.config/enscrive/license.jwt`.
///
/// Returns a descriptive error if HOME is unset and no override is provided.
pub fn resolve_license_path() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("ENSCRIVE_LICENSE_PATH") {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    let home = std::env::var("HOME").map_err(|_| {
        "cannot resolve license path: HOME is unset; set ENSCRIVE_LICENSE_PATH to override"
            .to_string()
    })?;
    Ok(PathBuf::from(home)
        .join(".config")
        .join("enscrive")
        .join("license.jwt"))
}

/// Decoded JWT claims (subset we care about).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LicenseClaims {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seats: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license_id: Option<String>,
    // Capture any extra fields too.
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, Value>,
}

/// Decode a JWT payload **without signature verification**.
///
/// Splits on `.`, takes the second segment, base64url-decodes it, and
/// deserializes the JSON. Returns `Err` if the JWT is structurally invalid
/// or the payload is not valid JSON.
pub fn decode_jwt_payload_unverified(jwt: &str) -> Result<LicenseClaims, String> {
    let parts: Vec<&str> = jwt.trim().splitn(3, '.').collect();
    if parts.len() != 3 {
        return Err(format!(
            "invalid JWT structure: expected 3 dot-separated segments, got {}",
            parts.len()
        ));
    }
    let payload_b64 = parts[1];
    // base64url decode — add padding as needed.
    let padded = {
        let mut s = payload_b64.to_string();
        match s.len() % 4 {
            2 => s.push_str("=="),
            3 => s.push('='),
            _ => {}
        }
        s
    };
    let bytes = base64_decode_url(&padded)
        .map_err(|e| format!("base64url decode of JWT payload failed: {e}"))?;
    let claims: LicenseClaims = serde_json::from_slice(&bytes)
        .map_err(|e| format!("JWT payload is not valid JSON: {e}"))?;
    Ok(claims)
}

/// Minimal base64url decoder (no external crate dependency).
fn base64_decode_url(s: &str) -> Result<Vec<u8>, String> {
    // Replace URL-safe chars with standard base64.
    let standard = s.replace('-', "+").replace('_', "/");
    use std::io::Read;
    // Use the stdlib's built-in base64 decoder via a simple implementation.
    base64_decode_standard(&standard)
}

fn base64_decode_standard(s: &str) -> Result<Vec<u8>, String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity((bytes.len() * 3) / 4 + 3);
    let mut buf = [0u8; 4];
    let mut buf_len = 0;

    fn decode_char(c: u8) -> Result<u8, String> {
        match c {
            b'A'..=b'Z' => Ok(c - b'A'),
            b'a'..=b'z' => Ok(c - b'a' + 26),
            b'0'..=b'9' => Ok(c - b'0' + 52),
            b'+' => Ok(62),
            b'/' => Ok(63),
            b'=' => Ok(0), // padding
            _ => Err(format!("invalid base64 character: {c}")),
        }
    }

    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'=' {
            // padding: flush what we have
            break;
        }
        buf[buf_len] = decode_char(c)?;
        buf_len += 1;
        if buf_len == 4 {
            out.push((buf[0] << 2) | (buf[1] >> 4));
            out.push((buf[1] << 4) | (buf[2] >> 2));
            out.push((buf[2] << 6) | buf[3]);
            buf_len = 0;
        }
        i += 1;
    }
    // Handle remaining.
    match buf_len {
        2 => out.push((buf[0] << 2) | (buf[1] >> 4)),
        3 => {
            out.push((buf[0] << 2) | (buf[1] >> 4));
            out.push((buf[1] << 4) | (buf[2] >> 2));
        }
        _ => {}
    }
    Ok(out)
}

/// Write the JWT to the resolved license path with 0600 permissions on Unix.
/// Returns the absolute path written (as String).
pub fn write_license_jwt(jwt: &str) -> Result<String, String> {
    let trimmed = jwt.trim();
    if trimmed.is_empty() {
        return Err("license JWT is empty".to_string());
    }

    let path = resolve_license_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("create license dir {}: {e}", parent.display()))?;
    }
    fs::write(&path, trimmed.as_bytes())
        .map_err(|e| format!("write license to {}: {e}", path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        fs::set_permissions(&path, perms)
            .map_err(|e| format!("set permissions 0600 on {}: {e}", path.display()))?;
    }

    #[cfg(not(unix))]
    {
        eprintln!(
            "enscrive: warning — running on non-Unix; file permissions on {} were not restricted to 0600.",
            path.display()
        );
    }

    Ok(path.display().to_string())
}

/// Read the JWT from the resolved license path. Returns `None` if absent.
pub fn read_license_jwt() -> Result<Option<String>, String> {
    let path = resolve_license_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("read license from {}: {e}", path.display()))?;
    Ok(Some(raw.trim().to_string()))
}

/// Remove the license file. Returns `true` if deleted, `false` if absent.
pub fn remove_license_file() -> Result<bool, String> {
    let path = resolve_license_path()?;
    if !path.exists() {
        return Ok(false);
    }
    fs::remove_file(&path)
        .map_err(|e| format!("remove license file {}: {e}", path.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_real_jwt_structure() {
        // A syntactically valid (but fake) JWT.
        // payload: {"plan":"professional","seats":5,"tenant_id":"t-123","expires_at":"2027-01-01T00:00:00Z","license_id":"lic-abc"}
        let payload_json = r#"{"plan":"professional","seats":5,"tenant_id":"t-123","expires_at":"2027-01-01T00:00:00Z","license_id":"lic-abc"}"#;
        let b64 = base64_encode_url(payload_json.as_bytes());
        let jwt = format!("eyJhbGciOiJSUzI1NiJ9.{b64}.fakesig");
        let claims = decode_jwt_payload_unverified(&jwt).expect("decode ok");
        assert_eq!(claims.plan.as_deref(), Some("professional"));
        assert_eq!(claims.tenant_id.as_deref(), Some("t-123"));
        assert_eq!(claims.license_id.as_deref(), Some("lic-abc"));
        assert_eq!(claims.expires_at.as_deref(), Some("2027-01-01T00:00:00Z"));
    }

    #[test]
    fn decode_invalid_jwt_errors() {
        let result = decode_jwt_payload_unverified("notajwt");
        assert!(result.is_err());
    }

    // helper for tests only
    fn base64_encode_url(data: &[u8]) -> String {
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        let mut i = 0;
        while i + 2 < data.len() {
            let b0 = data[i] as usize;
            let b1 = data[i + 1] as usize;
            let b2 = data[i + 2] as usize;
            out.push(CHARS[b0 >> 2] as char);
            out.push(CHARS[((b0 & 3) << 4) | (b1 >> 4)] as char);
            out.push(CHARS[((b1 & 0xf) << 2) | (b2 >> 6)] as char);
            out.push(CHARS[b2 & 0x3f] as char);
            i += 3;
        }
        match data.len() - i {
            1 => {
                let b0 = data[i] as usize;
                out.push(CHARS[b0 >> 2] as char);
                out.push(CHARS[(b0 & 3) << 4] as char);
                out.push_str("==");
            }
            2 => {
                let b0 = data[i] as usize;
                let b1 = data[i + 1] as usize;
                out.push(CHARS[b0 >> 2] as char);
                out.push(CHARS[((b0 & 3) << 4) | (b1 >> 4)] as char);
                out.push(CHARS[(b1 & 0xf) << 2] as char);
                out.push('=');
            }
            _ => {}
        }
        // URL-safe: replace + and /
        out.replace('+', "-").replace('/', "_")
    }
}
