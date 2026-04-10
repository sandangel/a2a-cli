package output

// sanitizeForTerminal strips characters from untrusted text that are dangerous
// in terminal output: ASCII control characters (except \n and \t which aid
// readability), bidi overrides, zero-width chars, and Unicode line/paragraph
// separators. This prevents escape-sequence injection and bidi spoofing from
// adversarial agent responses.
func sanitizeForTerminal(text string) string {
	out := make([]rune, 0, len(text))
	for _, c := range text {
		if c == '\n' || c == '\t' {
			out = append(out, c)
			continue
		}
		if c < 0x20 || c == 0x7F || (c >= 0x80 && c <= 0x9F) {
			// ASCII control chars and C1 control chars (e.g. CSI U+009B)
			continue
		}
		if isDangerousUnicode(c) {
			continue
		}
		out = append(out, c)
	}
	return string(out)
}

// isDangerousUnicode returns true for Unicode characters that can manipulate
// terminal rendering or aid in prompt injection:
//   - Zero-width chars: ZWSP, ZWNJ, ZWJ, BOM
//   - Bidi overrides: LRE, RLE, PDF, LRO, RLO
//   - Line/paragraph separators
//   - Directional isolates: LRI, RLI, FSI, PDI
func isDangerousUnicode(c rune) bool {
	switch {
	case c >= 0x200B && c <= 0x200D: // ZWSP, ZWNJ, ZWJ
		return true
	case c == 0xFEFF: // BOM / ZWNBSP
		return true
	case c >= 0x202A && c <= 0x202E: // LRE, RLE, PDF, LRO, RLO
		return true
	case c >= 0x2028 && c <= 0x2029: // line separator, paragraph separator
		return true
	case c >= 0x2066 && c <= 0x2069: // LRI, RLI, FSI, PDI
		return true
	}
	return false
}
