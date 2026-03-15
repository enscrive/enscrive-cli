use serde::Serialize;
use std::fmt;
use std::process;

/// Failure classifications per CLAUDE.md / validation strategy.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum FailureClass {
    #[serde(rename = "FAIL_BUG")]
    Bug,
    #[serde(rename = "FAIL_UNSUPPORTED")]
    Unsupported,
    #[serde(rename = "FAIL_UNIMPLEMENTED")]
    Unimplemented,
    #[serde(rename = "FAIL_FALSE_CLAIM")]
    FalseClaim,
}

impl fmt::Display for FailureClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bug => write!(f, "FAIL_BUG"),
            Self::Unsupported => write!(f, "FAIL_UNSUPPORTED"),
            Self::Unimplemented => write!(f, "FAIL_UNIMPLEMENTED"),
            Self::FalseClaim => write!(f, "FAIL_FALSE_CLAIM"),
        }
    }
}

/// Exit codes.
pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_FAILURE: i32 = 1;
pub const EXIT_UNSUPPORTED: i32 = 2;
pub const EXIT_CONFIG: i32 = 3;

/// Output format selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
}

/// Structured CLI response envelope.
#[derive(Debug, Serialize)]
pub struct CliResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_class: Option<FailureClass>,
    pub exit_code: i32,
}

impl CliResponse {
    pub fn success(command: &str, data: serde_json::Value) -> Self {
        Self {
            ok: true,
            command: Some(command.to_string()),
            data: Some(data),
            error: None,
            failure_class: None,
            exit_code: EXIT_SUCCESS,
        }
    }

    pub fn fail(command: &str, error: String, class: FailureClass, exit_code: i32) -> Self {
        Self {
            ok: false,
            command: Some(command.to_string()),
            data: None,
            error: Some(error),
            failure_class: Some(class),
            exit_code,
        }
    }

    pub fn unsupported(command: &str, message: &str) -> Self {
        Self::fail(
            command,
            message.to_string(),
            FailureClass::Unsupported,
            EXIT_UNSUPPORTED,
        )
    }

    /// Print and exit with the appropriate code.
    pub fn emit(self, format: OutputFormat) -> ! {
        let code = self.exit_code;
        match format {
            OutputFormat::Json => {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&self).expect("serialize response")
                );
            }
            OutputFormat::Human => {
                if self.ok {
                    if let Some(data) = &self.data {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(data).expect("serialize data")
                        );
                    }
                } else {
                    eprintln!(
                        "[{}] {}",
                        self.failure_class
                            .as_ref()
                            .map_or("ERROR".to_string(), |c| c.to_string()),
                        self.error.as_deref().unwrap_or("unknown error")
                    );
                }
            }
        }
        process::exit(code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_response_shape() {
        let r = CliResponse::success("search", serde_json::json!({"results": []}));
        assert!(r.ok);
        assert_eq!(r.exit_code, EXIT_SUCCESS);
        assert!(r.failure_class.is_none());
        assert_eq!(r.command.as_deref(), Some("search"));
    }

    #[test]
    fn unsupported_response_shape() {
        let r = CliResponse::unsupported(
            "evals run-beir",
            "evals namespace is not yet available on public /v1",
        );
        assert!(!r.ok);
        assert_eq!(r.exit_code, EXIT_UNSUPPORTED);
        assert_eq!(r.failure_class, Some(FailureClass::Unsupported));
    }

    #[test]
    fn failure_class_display() {
        assert_eq!(FailureClass::Bug.to_string(), "FAIL_BUG");
        assert_eq!(FailureClass::Unsupported.to_string(), "FAIL_UNSUPPORTED");
        assert_eq!(
            FailureClass::Unimplemented.to_string(),
            "FAIL_UNIMPLEMENTED"
        );
        assert_eq!(FailureClass::FalseClaim.to_string(), "FAIL_FALSE_CLAIM");
    }

    #[test]
    fn failure_class_serde() {
        let r = CliResponse::unsupported("test", "msg");
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["failure_class"], "FAIL_UNSUPPORTED");
    }

    #[test]
    fn success_json_omits_error_fields() {
        let r = CliResponse::success("search", serde_json::json!({}));
        let json = serde_json::to_value(&r).unwrap();
        assert!(json.get("error").is_none());
        assert!(json.get("failure_class").is_none());
    }

    #[test]
    fn fail_json_omits_data() {
        let r = CliResponse::unsupported("cmd", "reason");
        let json = serde_json::to_value(&r).unwrap();
        assert!(json.get("data").is_none());
    }
}
