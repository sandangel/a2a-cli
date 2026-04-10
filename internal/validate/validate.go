// Package validate provides input validation helpers that harden CLI inputs
// against adversarial or accidentally malformed values — especially important
// when the CLI is invoked by an LLM agent rather than a human operator.
package validate

import (
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"strings"
)

// --------------------------------------------------------------------------
// Character-level validation
// --------------------------------------------------------------------------

// DangerousInput returns an error if s contains characters that are unsafe
// at the CLI boundary: ASCII control chars, C1 control chars (U+0080-U+009F),
// DEL (U+007F), bidi overrides, zero-width chars, or Unicode line/paragraph
// separators. Applied to all flag values to prevent injection attacks.
func DangerousInput(s, field string) error {
	for _, c := range s {
		if c < 0x20 || c == 0x7F || (c >= 0x80 && c <= 0x9F) {
			return fmt.Errorf("%s contains invalid control characters", field)
		}
		if isDangerousUnicode(c) {
			return fmt.Errorf("%s contains invalid Unicode characters (bidi/zero-width)", field)
		}
	}
	return nil
}

// MessageText validates user-supplied message content. More permissive than
// DangerousInput — allows printable Unicode freely but still rejects null
// bytes, C0/C1 control chars (except \n and \t), and bidi overrides.
func MessageText(s, field string) error {
	for _, c := range s {
		if c == '\n' || c == '\t' {
			continue // allow for multi-line messages
		}
		if c < 0x20 || c == 0x7F || (c >= 0x80 && c <= 0x9F) {
			return fmt.Errorf("%s contains invalid control characters", field)
		}
		if isDangerousUnicode(c) {
			return fmt.Errorf("%s contains bidi override or zero-width Unicode characters", field)
		}
	}
	return nil
}

// Identifier validates an opaque ID (task ID, context ID, message ID) that
// will be used in API calls. Rejects path traversal, control chars, dangerous
// Unicode, and URL-special characters that could cause injection.
func Identifier(s, field string) error {
	if s == "" {
		return fmt.Errorf("%s must not be empty", field)
	}
	if err := DangerousInput(s, field); err != nil {
		return err
	}
	// Reject path traversal segments.
	for _, seg := range strings.Split(s, "/") {
		if seg == ".." {
			return fmt.Errorf("%s must not contain path traversal (..): %s", field, s)
		}
	}
	// Reject characters that could inject query params, fragments, or
	// URL-encoded bypasses (e.g. %2e%2e for ..).
	for _, c := range "?#%" {
		if strings.ContainsRune(s, c) {
			return fmt.Errorf("%s must not contain %q: %s", field, c, s)
		}
	}
	return nil
}

// AgentURL validates a URL provided as the agent endpoint. Must be HTTP or
// HTTPS, no null bytes, and a parseable absolute URL.
func AgentURL(s string) error {
	if s == "" {
		return fmt.Errorf("agent URL must not be empty")
	}
	if err := DangerousInput(s, "--agent"); err != nil {
		return err
	}
	u, err := url.Parse(s)
	if err != nil {
		return fmt.Errorf("invalid agent URL %q: %w", s, err)
	}
	if u.Scheme != "http" && u.Scheme != "https" {
		return fmt.Errorf("agent URL must use http or https scheme, got %q", u.Scheme)
	}
	if u.Host == "" {
		return fmt.Errorf("agent URL must include a host: %s", s)
	}
	return nil
}

// --------------------------------------------------------------------------
// Path validation
// --------------------------------------------------------------------------

// SafeOutputDir validates that dir is safe for writing output files.
//
// Rules (matching gws validate_safe_output_dir):
//   - No control characters or dangerous Unicode
//   - Relative paths only (absolute paths rejected)
//   - Resolved path must stay within CWD (symlink-safe)
//   - Non-existing paths are handled via normalize_non_existing logic
func SafeOutputDir(dir string) error {
	if dir == "" {
		return fmt.Errorf("output directory must not be empty")
	}
	if err := DangerousInput(dir, "--output-dir"); err != nil {
		return err
	}
	p := filepath.FromSlash(dir)
	if filepath.IsAbs(p) {
		return fmt.Errorf("--output-dir must be a relative path, got absolute path %q", dir)
	}
	return checkUnderCWD(p, "--output-dir")
}

// SafeFilePath validates a file path (e.g. --output, --upload). Absolute paths
// are allowed (reading from a known location is legitimate) but the resolved
// path must remain within CWD to prevent traversal escapes.
func SafeFilePath(path, field string) error {
	if path == "" {
		return fmt.Errorf("%s must not be empty", field)
	}
	if err := DangerousInput(path, field); err != nil {
		return err
	}
	p := filepath.FromSlash(path)
	if !filepath.IsAbs(p) {
		p = filepath.Join(".", p)
	}
	return checkUnderCWD(p, field)
}

// checkUnderCWD ensures that p (after canonicalization or normalization)
// resolves to a path within the current working directory.
func checkUnderCWD(p, field string) error {
	cwd, err := os.Getwd()
	if err != nil {
		return fmt.Errorf("cannot determine current directory: %w", err)
	}
	canonCWD, err := filepath.EvalSymlinks(cwd)
	if err != nil {
		canonCWD = filepath.Clean(cwd)
	}

	resolved := filepath.Join(cwd, p)
	if filepath.IsAbs(p) {
		resolved = p
	}

	var canonical string
	if _, statErr := os.Lstat(resolved); statErr == nil {
		// Path exists — follow symlinks.
		canonical, err = filepath.EvalSymlinks(resolved)
		if err != nil {
			canonical = filepath.Clean(resolved)
		}
	} else {
		// Path doesn't exist yet — canonicalize the existing prefix, then
		// apply normalize_dotdot to resolve any remaining .. components.
		canonical = normalizeNonExisting(resolved)
	}

	// Ensure canonical is under CWD.
	rel, err := filepath.Rel(canonCWD, canonical)
	if err != nil || strings.HasPrefix(rel, "..") {
		return fmt.Errorf("%s %q resolves outside the current directory", field, p)
	}
	return nil
}

// normalizeNonExisting resolves a path that may not fully exist by
// canonicalizing the deepest existing prefix, then appending remaining
// segments and resolving all .. components without touching the filesystem.
// This mirrors gws's normalize_non_existing + normalize_dotdot.
func normalizeNonExisting(p string) string {
	current := filepath.Clean(p)
	var remaining []string

	for {
		if _, err := os.Lstat(current); err == nil {
			// This prefix exists.
			if canon, err := filepath.EvalSymlinks(current); err == nil {
				current = canon
			}
			break
		}
		parent := filepath.Dir(current)
		if parent == current {
			// Reached filesystem root without finding an existing prefix.
			break
		}
		remaining = append([]string{filepath.Base(current)}, remaining...)
		current = parent
	}

	result := current
	for _, seg := range remaining {
		result = filepath.Join(result, seg)
	}
	// Resolve any remaining .. components.
	return normalizeDotDot(result)
}

// normalizeDotDot resolves . and .. components in a path without touching
// the filesystem, identical to gws's normalize_dotdot.
func normalizeDotDot(p string) string {
	var out []string
	vol := filepath.VolumeName(p)
	rest := p[len(vol):]
	for _, seg := range strings.Split(rest, string(filepath.Separator)) {
		switch seg {
		case "", ".":
			// skip
		case "..":
			if len(out) > 0 {
				out = out[:len(out)-1]
			}
		default:
			out = append(out, seg)
		}
	}
	result := vol + string(filepath.Separator) + strings.Join(out, string(filepath.Separator))
	if !filepath.IsAbs(p) {
		result = strings.TrimPrefix(result, string(filepath.Separator))
	}
	return result
}

// --------------------------------------------------------------------------
// Dangerous Unicode detector (shared with output.sanitizeForTerminal)
// --------------------------------------------------------------------------

func isDangerousUnicode(c rune) bool {
	switch {
	case c >= 0x200B && c <= 0x200D: // ZWSP, ZWNJ, ZWJ
		return true
	case c == 0xFEFF: // BOM
		return true
	case c >= 0x202A && c <= 0x202E: // LRE, RLE, PDF, LRO, RLO
		return true
	case c >= 0x2028 && c <= 0x2029: // line/paragraph separators
		return true
	case c >= 0x2066 && c <= 0x2069: // LRI, RLI, FSI, PDI
		return true
	}
	return false
}
