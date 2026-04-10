package validate_test

import (
	"os"
	"path/filepath"
	"testing"

	"genai.stargate.toyota/agc/internal/validate"
)

// --------------------------------------------------------------------------
// DangerousInput
// --------------------------------------------------------------------------

func TestDangerousInput_Clean(t *testing.T) {
	for _, s := range []string{"hello world", "task-abc123", "日本語", "café", "αβγ"} {
		if err := validate.DangerousInput(s, "test"); err != nil {
			t.Errorf("clean input %q should pass, got: %v", s, err)
		}
	}
}

func TestDangerousInput_ControlChars(t *testing.T) {
	cases := []struct {
		s    string
		desc string
	}{
		{"foo\x00bar", "null byte"},
		{"foo\nbar", "newline"},
		{"foo\tbar", "tab"},
		{"foo\x1Bbar", "ESC"},
		{"foo\x7Fbar", "DEL"},
		{"foo\u009Bbar", "C1 CSI (U+009B, valid UTF-8 \xc2\x9b)"},
	}
	for _, tc := range cases {
		if err := validate.DangerousInput(tc.s, "test"); err == nil {
			t.Errorf("%s in %q should fail", tc.desc, tc.s)
		}
	}
}

func TestDangerousInput_DangerousUnicode(t *testing.T) {
	for _, s := range []string{
		"foo\u200Bbar",  // ZWSP
		"foo\u200Dbar",  // ZWJ
		"foo\uFEFFbar",  // BOM
		"foo\u202Ebar",  // RLO
		"foo\u2028bar",  // line separator
		"foo\u2029bar",  // paragraph separator
		"foo\u2066bar",  // LRI
	} {
		if err := validate.DangerousInput(s, "test"); err == nil {
			t.Errorf("dangerous unicode in %q should fail", s)
		}
	}
}

// --------------------------------------------------------------------------
// MessageText
// --------------------------------------------------------------------------

func TestMessageText_AllowsNewlineAndTab(t *testing.T) {
	if err := validate.MessageText("line1\nline2\ttab", "msg"); err != nil {
		t.Errorf("newline/tab should be allowed in message text: %v", err)
	}
}

func TestMessageText_RejectsNullByte(t *testing.T) {
	if err := validate.MessageText("foo\x00bar", "msg"); err == nil {
		t.Error("null byte should be rejected")
	}
}

func TestMessageText_RejectsBidi(t *testing.T) {
	if err := validate.MessageText("foo\u202Ebar", "msg"); err == nil {
		t.Error("RLO bidi override should be rejected")
	}
}

// --------------------------------------------------------------------------
// Identifier
// --------------------------------------------------------------------------

func TestIdentifier_Valid(t *testing.T) {
	for _, s := range []string{"task-abc123", "ctx_xyz", "01917c5e-37c2"} {
		if err := validate.Identifier(s, "id"); err != nil {
			t.Errorf("valid id %q should pass: %v", s, err)
		}
	}
}

func TestIdentifier_Empty(t *testing.T) {
	if err := validate.Identifier("", "id"); err == nil {
		t.Error("empty identifier should fail")
	}
}

func TestIdentifier_PathTraversal(t *testing.T) {
	for _, s := range []string{"../../etc/passwd", "tasks/../admin", ".."} {
		if err := validate.Identifier(s, "id"); err == nil {
			t.Errorf("path traversal %q should fail", s)
		}
	}
}

func TestIdentifier_URLInjection(t *testing.T) {
	for _, s := range []string{"id?key=val", "id#frag", "id%2e%2e"} {
		if err := validate.Identifier(s, "id"); err == nil {
			t.Errorf("URL injection %q should fail", s)
		}
	}
}

func TestIdentifier_ControlChars(t *testing.T) {
	if err := validate.Identifier("id\x00null", "id"); err == nil {
		t.Error("null byte in identifier should fail")
	}
}

// --------------------------------------------------------------------------
// AgentURL
// --------------------------------------------------------------------------

func TestAgentURL_Valid(t *testing.T) {
	for _, s := range []string{
		"http://localhost:8080",
		"https://my-agent.example.com",
		"http://127.0.0.1:9001/v1",
	} {
		if err := validate.AgentURL(s); err != nil {
			t.Errorf("valid agent URL %q should pass: %v", s, err)
		}
	}
}

func TestAgentURL_Empty(t *testing.T) {
	if err := validate.AgentURL(""); err == nil {
		t.Error("empty URL should fail")
	}
}

func TestAgentURL_InvalidScheme(t *testing.T) {
	for _, s := range []string{"ftp://host", "file:///etc/passwd", "javascript:alert(1)"} {
		if err := validate.AgentURL(s); err == nil {
			t.Errorf("invalid scheme %q should fail", s)
		}
	}
}

func TestAgentURL_ControlChars(t *testing.T) {
	if err := validate.AgentURL("http://host\x00.example.com"); err == nil {
		t.Error("URL with null byte should fail")
	}
}

func TestAgentURL_NoHost(t *testing.T) {
	if err := validate.AgentURL("http://"); err == nil {
		t.Error("URL without host should fail")
	}
}

// --------------------------------------------------------------------------
// SafeOutputDir
// --------------------------------------------------------------------------

func TestSafeOutputDir_RelativeSubdir(t *testing.T) {
	dir := t.TempDir()
	sub := filepath.Join(dir, "output")
	if err := os.MkdirAll(sub, 0o755); err != nil {
		t.Fatal(err)
	}
	orig, _ := os.Getwd()
	defer os.Chdir(orig) //nolint:errcheck
	os.Chdir(dir)        //nolint:errcheck

	if err := validate.SafeOutputDir("output"); err != nil {
		t.Errorf("relative subdir should pass: %v", err)
	}
}

func TestSafeOutputDir_NonExistingSubdir(t *testing.T) {
	dir := t.TempDir()
	orig, _ := os.Getwd()
	defer os.Chdir(orig) //nolint:errcheck
	os.Chdir(dir)        //nolint:errcheck

	if err := validate.SafeOutputDir("new/nested/dir"); err != nil {
		t.Errorf("non-existing subdir should pass: %v", err)
	}
}

func TestSafeOutputDir_RejectsAbsolute(t *testing.T) {
	if err := validate.SafeOutputDir("/tmp/evil"); err == nil {
		t.Error("absolute path should be rejected")
	}
}

func TestSafeOutputDir_RejectsTraversal(t *testing.T) {
	dir := t.TempDir()
	orig, _ := os.Getwd()
	defer os.Chdir(orig) //nolint:errcheck
	os.Chdir(dir)        //nolint:errcheck

	if err := validate.SafeOutputDir("../../.ssh"); err == nil {
		t.Error("traversal above CWD should be rejected")
	}
}

func TestSafeOutputDir_RejectsNullByte(t *testing.T) {
	if err := validate.SafeOutputDir("foo\x00bar"); err == nil {
		t.Error("null byte in output dir should be rejected")
	}
}

func TestSafeOutputDir_RejectsBidiOverride(t *testing.T) {
	if err := validate.SafeOutputDir("foo\u202Ebar"); err == nil {
		t.Error("bidi override in output dir should be rejected")
	}
}

func TestSafeOutputDir_RejectsTraversalViaNonExistentPrefix(t *testing.T) {
	// Regression: doesnt_exist/../../etc/passwd bypasses naive starts_with check.
	dir := t.TempDir()
	orig, _ := os.Getwd()
	defer os.Chdir(orig) //nolint:errcheck
	os.Chdir(dir)        //nolint:errcheck

	if err := validate.SafeOutputDir("doesnt_exist/../../etc/passwd"); err == nil {
		t.Error("traversal via non-existent prefix should be rejected")
	}
}

func TestSafeOutputDir_RejectsEmpty(t *testing.T) {
	if err := validate.SafeOutputDir(""); err == nil {
		t.Error("empty dir should be rejected")
	}
}
