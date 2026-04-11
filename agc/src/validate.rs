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
