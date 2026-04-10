// Package output provides human-friendly and JSON rendering for agc responses.
package output

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
	"strings"

	"gopkg.in/yaml.v3"
)

// Printer writes output to stdout (results) and stderr (errors).
// In human mode (default) responses are rendered as readable text.
// With --json they are rendered as normalized JSON.
type Printer struct {
	Fields string // --fields filter applied before output
	Format string
	Out    io.Writer
	ErrOut io.Writer
}

// New creates a Printer for stdout/stderr.
func New(format, fields string) *Printer {
	return &Printer{Format: format, Fields: fields, Out: os.Stdout, ErrOut: os.Stderr}
}

// --------------------------------------------------------------------------
// AgentResponse rendering
// --------------------------------------------------------------------------

// PrintResponse is the primary method for AgentResponse output.
//   - Human mode, single agent: readable text (just the answer)
//   - Human mode, multi-agent: separator per agent then text
//   - JSON mode, single agent: pretty-printed JSON
//   - JSON mode, multi-agent: compact NDJSON (one line per agent)
func (p *Printer) PrintResponse(resp AgentResponse, multiAgent bool) error {
	if p.Format == "json" {
		if multiAgent {
			return p.PrintJSON(resp)
		}
		return p.prettyJSON(resp)
	}
	return p.printHuman(resp, multiAgent)
}

// PrintJSON writes v as compact JSON (one line). --fields is applied if set.
func (p *Printer) PrintJSON(v any) error {
	b, err := json.Marshal(p.applyFields(v))
	if err != nil {
		return err
	}
	_, err = fmt.Fprintln(p.Out, string(b))
	return err
}

// Print writes v as pretty JSON or human text. --fields is applied in json mode.
func (p *Printer) Print(v any) error {
	if p.Format == "json" {
		return p.prettyJSON(p.applyFields(v))
	}
	// Human fallback: pretty JSON (generic data has no human renderer)
	return p.prettyJSON(v)
}

// PrintLine writes a sanitized line to stdout.
func (p *Printer) PrintLine(s string) {
	fmt.Fprintln(p.Out, sanitizeForTerminal(s))
}

// PrintError writes an error to stderr.
func (p *Printer) PrintError(err error) {
	msg := sanitizeForTerminal(err.Error())
	if p.Format == "json" {
		b, _ := json.Marshal(map[string]string{"error": msg})
		fmt.Fprintln(p.ErrOut, string(b))
	} else {
		fmt.Fprintf(p.ErrOut, "error: %s\n", msg)
	}
}

// --------------------------------------------------------------------------
// Human-friendly rendering
// --------------------------------------------------------------------------

func (p *Printer) printHuman(r AgentResponse, multiAgent bool) error {
	if multiAgent {
		// Print a header separator for each agent.
		label := r.Agent
		if r.AgentURL != "" && r.AgentURL != r.Agent {
			label = fmt.Sprintf("%s (%s)", r.Agent, r.AgentURL)
		}
		header := fmt.Sprintf("── %s ", label)
		if len(header) < 60 {
			header += strings.Repeat("─", 60-len(header))
		}
		p.PrintLine(header)
	}

	switch r.Type {
	case "message", "task_completed":
		p.printContent(r)

	case "task_failed":
		fmt.Fprintf(p.Out, "FAILED")
		if r.Text != "" {
			fmt.Fprintf(p.Out, ": %s", sanitizeForTerminal(r.Text))
		}
		fmt.Fprintln(p.Out)

	case "task_canceled", "task_rejected":
		fmt.Fprintf(p.Out, "%s", strings.ToUpper(strings.TrimPrefix(r.Type, "task_")))
		if r.Text != "" {
			fmt.Fprintf(p.Out, ": %s", sanitizeForTerminal(r.Text))
		}
		fmt.Fprintln(p.Out)

	case "task_working", "task_submitted":
		if r.Text != "" {
			p.PrintLine(r.Text)
		}
		if r.TaskID != "" {
			fmt.Fprintf(p.Out, "\nTask still in progress (id: %s)\n", sanitizeForTerminal(string(r.TaskID)))
			fmt.Fprintf(p.Out, "  Check status: agc task get --id %s\n", sanitizeForTerminal(string(r.TaskID)))
		}

	case "input_required":
		if r.Text != "" {
			fmt.Fprintf(p.Out, "Input required: %s\n", sanitizeForTerminal(r.Text))
		} else {
			fmt.Fprintf(p.Out, "Input required\n")
		}
		if r.TaskID != "" {
			fmt.Fprintf(p.Out, "  Reply: agc --agent %s send --task-id %s \"your response\"\n",
				sanitizeForTerminal(r.Agent), sanitizeForTerminal(string(r.TaskID)))
		}

	case "auth_required":
		// Agent needs a credential delivered out-of-band (e.g. OAuth for MCP server).
		// The agent goroutine keeps running in the background waiting for the callback.
		// The auth URL / instructions are in the status message text.
		fmt.Fprintln(p.Out, "Authentication required")
		if r.Text != "" {
			fmt.Fprintln(p.Out)
			fmt.Fprintln(p.Out, sanitizeForTerminal(r.Text))
		}
		if r.TaskID != "" {
			fmt.Fprintf(p.Out, "\nTask running in background — will complete after authentication.\n")
			fmt.Fprintf(p.Out, "Check: agc --agent %s task get --id %s\n",
				sanitizeForTerminal(r.Agent), sanitizeForTerminal(string(r.TaskID)))
		}

	case "error":
		fmt.Fprintf(p.ErrOut, "error [%s]: %s\n",
			sanitizeForTerminal(r.Agent), sanitizeForTerminal(r.Error))

	default:
		p.printContent(r)
	}

	if multiAgent {
		fmt.Fprintln(p.Out) // blank line between agents
	}
	return nil
}

// printContent prints the text body and any artifacts.
func (p *Printer) printContent(r AgentResponse) {
	if r.Text != "" {
		fmt.Fprintln(p.Out, sanitizeForTerminal(r.Text))
	}
	for _, art := range r.Artifacts {
		if art.Name != "" {
			fmt.Fprintf(p.Out, "\n── %s ──\n", sanitizeForTerminal(art.Name))
		}
		if art.Text != "" {
			fmt.Fprintln(p.Out, sanitizeForTerminal(art.Text))
		}
	}
	if len(r.Artifacts) == 0 && r.Text == "" && r.TaskID != "" {
		fmt.Fprintf(p.Out, "(no content — task: %s)\n", sanitizeForTerminal(string(r.TaskID)))
	}
}

// --------------------------------------------------------------------------
// Table formatter (for task list, card, config — generic structs)
// --------------------------------------------------------------------------

// PrintTable renders a slice of maps as an aligned text table.
// headers lists the keys to display; rows is []map[string]string.
func (p *Printer) PrintTable(headers []string, rows [][]string) {
	widths := make([]int, len(headers))
	for i, h := range headers {
		widths[i] = len(h)
	}
	for _, row := range rows {
		for i, cell := range row {
			if i < len(widths) && len(cell) > widths[i] {
				widths[i] = min(len(cell), 60)
			}
		}
	}
	// Header
	for i, h := range headers {
		if i > 0 {
			fmt.Fprint(p.Out, "  ")
		}
		fmt.Fprintf(p.Out, "%-*s", widths[i], h)
	}
	fmt.Fprintln(p.Out)
	// Separator
	for i, w := range widths {
		if i > 0 {
			fmt.Fprint(p.Out, "  ")
		}
		fmt.Fprint(p.Out, strings.Repeat("─", w))
	}
	fmt.Fprintln(p.Out)
	// Rows
	for _, row := range rows {
		for i := range headers {
			if i > 0 {
				fmt.Fprint(p.Out, "  ")
			}
			cell := ""
			if i < len(row) {
				cell = sanitizeForTerminal(row[i])
				if len(cell) > widths[i] {
					runes := []rune(cell)
					cell = string(runes[:widths[i]-1]) + "…"
				}
			}
			fmt.Fprintf(p.Out, "%-*s", widths[i], cell)
		}
		fmt.Fprintln(p.Out)
	}
}

// --------------------------------------------------------------------------
// Internal helpers
// --------------------------------------------------------------------------

// applyFields runs FilterFields if --fields is set, otherwise returns v unchanged.
func (p *Printer) applyFields(v any) any {
	if p.Fields == "" {
		return v
	}
	// Round-trip through JSON to get map[string]any for uniform filtering.
	b, err := json.Marshal(v)
	if err != nil {
		return v
	}
	var m any
	if err := json.Unmarshal(b, &m); err != nil {
		return v
	}
	return FilterFields(m, p.Fields)
}

func (p *Printer) prettyJSON(v any) error {
	enc := json.NewEncoder(p.Out)
	enc.SetIndent("", "  ")
	return enc.Encode(v)
}

// Truncate shortens a string to maxLen runes, adding "…" if needed.
func Truncate(s string, maxLen int) string {
	s = strings.ReplaceAll(s, "\n", " ")
	runes := []rune(s)
	if len(runes) <= maxLen {
		return s
	}
	return string(runes[:maxLen-1]) + "…"
}

// NewTable creates a tablewriter-style table — kept for agent_cmd.go compat.
// Returns a simple function to add rows and render.
func NewTable(w io.Writer, headers []string) *SimpleTable {
	return &SimpleTable{w: w, headers: headers}
}

// SimpleTable is a minimal table renderer.
type SimpleTable struct {
	w       io.Writer
	headers []string
	rows    [][]string
}

func (t *SimpleTable) Append(row []string) { t.rows = append(t.rows, row) }

func (t *SimpleTable) Render() {
	p := &Printer{Out: t.w, ErrOut: os.Stderr}
	p.PrintTable(t.headers, t.rows)
}

// yaml export kept for config show
func MarshalYAML(v any) ([]byte, error) {
	raw, err := json.Marshal(v)
	if err != nil {
		return nil, err
	}
	var m any
	if err := json.Unmarshal(raw, &m); err != nil {
		return nil, err
	}
	return yaml.Marshal(m)
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}
