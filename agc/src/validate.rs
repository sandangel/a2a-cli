//! Input validation — thin wrappers over google_workspace::validate,
//! plus A2A-specific checks.
//!
//! Validated newtypes (`AgentAlias`, `AgentUrl`) encode the invariant in the
//! type: once constructed, callers don't need to re-validate.

use crate::error::{AgcError, Result};
pub use google_workspace::validate::is_dangerous_unicode;

// ── Validated newtypes ────────────────────────────────────────────────

/// A validated agent alias — not empty, no path separators, no control chars.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AgentAlias(String);

impl AgentAlias {
    pub fn new(s: impl Into<String>) -> Result<Self> {
        let s = s.into();
        validate_alias_str(&s)?;
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AgentAlias {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<str> for AgentAlias {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// A validated agent URL — must be `http://` or `https://`, no control chars.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentUrl(String);

impl AgentUrl {
    pub fn new(s: impl Into<String>) -> Result<Self> {
        let s = s.into();
        validate_url_str(&s)?;
        Ok(Self(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for AgentUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<str> for AgentUrl {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// ── Core validation logic (reused by newtypes and free functions) ─────

pub fn reject_dangerous_chars(value: &str, flag_name: &str) -> Result<()> {
    for c in value.chars() {
        if c.is_control() {
            return Err(AgcError::InvalidInput(format!(
                "{flag_name} contains invalid control characters"
            )));
        }
        if is_dangerous_unicode(c) {
            return Err(AgcError::InvalidInput(format!(
                "{flag_name} contains invalid Unicode characters"
            )));
        }
    }
    Ok(())
}

fn validate_url_str(url: &str) -> Result<()> {
    reject_dangerous_chars(url, "--agent")?;
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(AgcError::InvalidInput(format!(
            "agent URL must start with http:// or https://, got: {url}"
        )));
    }
    Ok(())
}

fn validate_alias_str(alias: &str) -> Result<()> {
    if alias.is_empty() {
        return Err(AgcError::InvalidInput(
            "alias must not be empty".to_string(),
        ));
    }
    reject_dangerous_chars(alias, "alias")?;
    if alias.contains('/') || alias.contains('\\') {
        return Err(AgcError::InvalidInput(format!(
            "alias must not contain path separators: {alias}"
        )));
    }
    Ok(())
}

// ── Free functions (kept for backward compat call sites) ─────────────

/// Validate an agent URL string; prefer `AgentUrl::new()` at boundaries.
pub fn validate_agent_url(url: &str) -> Result<()> {
    validate_url_str(url)
}

/// Validate an alias string; prefer `AgentAlias::new()` at boundaries.
pub fn validate_alias(alias: &str) -> Result<()> {
    validate_alias_str(alias)
}

/// Validate and normalise a raw `--agent` value (alias or URL).
///
/// This is the single entry point for all agent-ref validation — avoids
/// duplicating the `starts_with("http")` branch at every call site.
/// Returns the validated string unchanged so it can be forwarded to config
/// lookup without an extra allocation.
pub fn validate_agent_ref(s: &str) -> Result<String> {
    if s.starts_with("http://") || s.starts_with("https://") {
        AgentUrl::new(s)?;
    } else {
        AgentAlias::new(s)?;
    }
    Ok(s.to_string())
}

pub fn validate_message_text(text: &str) -> Result<()> {
    for c in text.chars() {
        if c == '\n' || c == '\t' {
            continue;
        }
        if c.is_control() {
            return Err(AgcError::InvalidInput(
                "message text contains invalid control characters".to_string(),
            ));
        }
        if is_dangerous_unicode(c) {
            return Err(AgcError::InvalidInput(
                "message text contains invalid Unicode characters".to_string(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── AgentAlias ────────────────────────────────────────────────────

    #[test]
    fn alias_newtype_valid() {
        assert!(AgentAlias::new("rover").is_ok());
    }

    #[test]
    fn alias_newtype_empty_rejected() {
        assert!(AgentAlias::new("").is_err());
    }

    #[test]
    fn alias_newtype_slash_rejected() {
        assert!(AgentAlias::new("foo/bar").is_err());
    }

    #[test]
    fn alias_newtype_as_str() {
        let a = AgentAlias::new("my-agent").unwrap();
        assert_eq!(a.as_str(), "my-agent");
    }

    // ── AgentUrl ──────────────────────────────────────────────────────

    #[test]
    fn url_newtype_https_valid() {
        assert!(AgentUrl::new("https://example.com/a2a").is_ok());
    }

    #[test]
    fn url_newtype_http_valid() {
        assert!(AgentUrl::new("http://localhost:8080").is_ok());
    }

    #[test]
    fn url_newtype_ftp_rejected() {
        assert!(AgentUrl::new("ftp://example.com").is_err());
    }

    #[test]
    fn url_newtype_bare_host_rejected() {
        assert!(AgentUrl::new("example.com").is_err());
    }

    #[test]
    fn url_newtype_as_str() {
        let u = AgentUrl::new("https://example.com").unwrap();
        assert_eq!(u.as_str(), "https://example.com");
    }

    // ── reject_dangerous_chars ────────────────────────────────────────

    #[test]
    fn reject_null_byte() {
        assert!(reject_dangerous_chars("foo\0bar", "flag").is_err());
    }

    #[test]
    fn reject_esc_control_char() {
        assert!(reject_dangerous_chars("foo\x1Bbar", "flag").is_err());
    }

    #[test]
    fn reject_bidi_override() {
        assert!(reject_dangerous_chars("foo\u{202E}bar", "flag").is_err());
    }

    #[test]
    fn allow_normal_ascii() {
        assert!(reject_dangerous_chars("hello world 123!", "flag").is_ok());
    }

    #[test]
    fn allow_unicode_letters() {
        assert!(reject_dangerous_chars("日本語テスト", "flag").is_ok());
    }

    // ── validate_agent_url ────────────────────────────────────────────

    #[test]
    fn agent_url_http_ok() {
        assert!(validate_agent_url("http://example.com").is_ok());
    }

    #[test]
    fn agent_url_https_with_path_ok() {
        assert!(validate_agent_url("https://example.com/a2a/agent").is_ok());
    }

    #[test]
    fn agent_url_rejects_ftp() {
        assert!(validate_agent_url("ftp://example.com").is_err());
    }

    #[test]
    fn agent_url_rejects_bare_host() {
        assert!(validate_agent_url("example.com").is_err());
    }

    #[test]
    fn agent_url_rejects_empty() {
        assert!(validate_agent_url("").is_err());
    }

    #[test]
    fn agent_url_rejects_control_char_in_path() {
        assert!(validate_agent_url("http://example.com/\x00path").is_err());
    }

    // ── validate_alias ────────────────────────────────────────────────

    #[test]
    fn alias_empty_rejected() {
        assert!(validate_alias("").is_err());
    }

    #[test]
    fn alias_forward_slash_rejected() {
        assert!(validate_alias("foo/bar").is_err());
    }

    #[test]
    fn alias_backslash_rejected() {
        assert!(validate_alias("foo\\bar").is_err());
    }

    #[test]
    fn alias_simple_ok() {
        assert!(validate_alias("my-agent").is_ok());
    }

    #[test]
    fn alias_with_numbers_ok() {
        assert!(validate_alias("agent42").is_ok());
    }

    // ── validate_message_text ─────────────────────────────────────────

    #[test]
    fn message_text_plain_ok() {
        assert!(validate_message_text("Hello, world!").is_ok());
    }

    #[test]
    fn message_text_newline_allowed() {
        assert!(validate_message_text("line1\nline2").is_ok());
    }

    #[test]
    fn message_text_tab_allowed() {
        assert!(validate_message_text("col1\tcol2").is_ok());
    }

    #[test]
    fn message_text_null_byte_rejected() {
        assert!(validate_message_text("foo\0bar").is_err());
    }

    #[test]
    fn message_text_bel_control_rejected() {
        assert!(validate_message_text("foo\x07bar").is_err());
    }

    #[test]
    fn message_text_bidi_override_rejected() {
        assert!(validate_message_text("foo\u{202E}bar").is_err());
    }

    #[test]
    fn message_text_unicode_ok() {
        assert!(validate_message_text("こんにちは 🎉").is_ok());
    }
}
