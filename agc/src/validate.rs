//! Input validation — thin wrappers over google_workspace::validate,
//! plus A2A-specific checks.

use crate::error::{AgcError, Result};
pub use google_workspace::validate::is_dangerous_unicode;

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

pub fn validate_agent_url(url: &str) -> Result<()> {
    reject_dangerous_chars(url, "--agent")?;
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(AgcError::InvalidInput(format!(
            "agent URL must start with http:// or https://, got: {url}"
        )));
    }
    Ok(())
}

pub fn validate_alias(alias: &str) -> Result<()> {
    if alias.is_empty() {
        return Err(AgcError::InvalidInput("alias must not be empty".to_string()));
    }
    reject_dangerous_chars(alias, "alias")?;
    if alias.contains('/') || alias.contains('\\') {
        return Err(AgcError::InvalidInput(format!(
            "alias must not contain path separators: {alias}"
        )));
    }
    Ok(())
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
        // U+202E RIGHT-TO-LEFT OVERRIDE
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
        // U+0007 BEL
        assert!(validate_message_text("foo\x07bar").is_err());
    }

    #[test]
    fn message_text_bidi_override_rejected() {
        // U+202E RIGHT-TO-LEFT OVERRIDE
        assert!(validate_message_text("foo\u{202E}bar").is_err());
    }

    #[test]
    fn message_text_unicode_ok() {
        assert!(validate_message_text("こんにちは 🎉").is_ok());
    }
}
