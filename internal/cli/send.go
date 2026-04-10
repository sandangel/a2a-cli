package cli

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"runtime"
	"strings"
	"sync"
	"time"

	"github.com/a2aproject/a2a-go/v2/a2a"
	"github.com/spf13/cobra"
	"genai.stargate.toyota/agc/internal/output"
	"genai.stargate.toyota/agc/internal/validate"
)

var (
	ErrInputRequired = errors.New("input required")
	ErrAuthRequired  = errors.New("auth required")
)

var sendFlags struct {
	contextID     string
	taskID        string
	historyLength int
	params        string // --params '{"message":{...}}' — full SendMessageRequest JSON
}

var sendCmd = &cobra.Command{
	Use:   "send [message]",
	Short: "Send a message to one or more agents",
	Long: `Send a message to an agent and wait for the response.

See: agc schema send — full SendMessageRequest schema
     agc schema task — understand the response

PARAMS FLAG
  --params overrides the text argument with a full SendMessageRequest JSON.
  This lets you set taskId, contextId, metadata, extensions, etc.
  Example:
    agc send --params '{"message":{"parts":[{"text":"hello"}],"taskId":"task-abc"}}'
    agc send --params '{"message":{"parts":[{"text":"go"}],"metadata":{"skill_id":"summarize"}}}'

INTERRUPTED STATES
  input_required  → In terminal: prompts you immediately.
                    With --format json: exits 2, AI replies with --task-id.
  auth_required   → In terminal: opens browser, keeps waiting.
                    With --format json: exits 3, AI asks human to authenticate.`,
	Example: `  agc send "What is today's date?"
  agc --format json send "Hello" | jq '.status.message.parts[0].text'

  # Full params (use agc schema send to understand the structure)
  agc send --params '{"message":{"parts":[{"text":"hi"}],"taskId":"task-abc"}}'

  # Multiple agents
  agc --all send "Status?"
  agc --agent prod --agent staging send "Health check?"`,
	Args: cobra.MaximumNArgs(1),
	RunE: runSend,
}

func init() {
	sendCmd.Flags().StringVar(&sendFlags.contextID, "context-id", "",
		"Existing context ID to continue a conversation")
	sendCmd.Flags().StringVar(&sendFlags.taskID, "task-id", "",
		"Existing task ID to continue")
	sendCmd.Flags().IntVar(&sendFlags.historyLength, "history-length", 0,
		"Number of history messages to include in the response")
	sendCmd.Flags().StringVar(&sendFlags.params, "params", "",
		"Full SendMessageRequest as JSON (overrides text arg — see: agc schema send)")
}

func runSend(cmd *cobra.Command, args []string) error {
	p := printer()

	var msgText string
	if sendFlags.params == "" {
		var msgErr error
		msgText, msgErr = resolveMessage(args, cmd.InOrStdin())
		if msgErr != nil {
			p.PrintError(msgErr)
			return msgErr
		}
		if valErr := validate.MessageText(msgText, "message"); valErr != nil {
			p.PrintError(valErr)
			return valErr
		}
	}
	if sendFlags.taskID != "" {
		if err := validate.Identifier(sendFlags.taskID, "--task-id"); err != nil {
			p.PrintError(err)
			return err
		}
	}
	if sendFlags.contextID != "" {
		if err := validate.Identifier(sendFlags.contextID, "--context-id"); err != nil {
			p.PrintError(err)
			return err
		}
	}

	targets, err := resolveTargets()
	if err != nil {
		p.PrintError(err)
		return err
	}

	multiAgent := len(targets) > 1
	timeout := resolvedTimeout()
	// In human/terminal mode: allow time for OAuth flows (10 min).
	// In json mode: AI handles timing, short timeout is fine.
	if !multiAgent && flags.format != "json" && timeout < 10*time.Minute {
		timeout = 10 * time.Minute
	}
	ctx, cancel := context.WithTimeout(cmd.Context(), timeout)
	defer cancel()

	if !multiAgent {
		return sendSingle(ctx, p, targets[0], msgText)
	}
	return sendParallel(ctx, p, targets, msgText)
}

// --------------------------------------------------------------------------
// Single-agent send — uses streaming internally.
// --------------------------------------------------------------------------

// collectText appends the first text found in msg.Parts to *dst if non-empty.
func collectText(msg *a2a.Message, dst *string) {
	if msg == nil {
		return
	}
	for _, part := range msg.Parts {
		if txt := part.Text(); txt != "" {
			*dst = txt
			return
		}
	}
}

// printFinal outputs the agent's answer.
// Human mode: print the plain text if available, otherwise the task JSON.
// JSON mode: always print the task, injecting lastText into the status message
// when the terminal event itself carries no message (Rover sends the answer in
// the last working event, not the completed event).
func printFinal(p *output.Printer, task *a2a.Task, lastText string) error {
	if flags.format != "json" {
		if lastText != "" {
			p.PrintLine(lastText)
			return nil
		}
		return p.Print(task)
	}
	// JSON mode — ensure the answer is in the task's status message.
	if lastText != "" && (task.Status.Message == nil || len(task.Status.Message.Parts) == 0) {
		task.Status.Message = a2a.NewMessage(a2a.MessageRoleAgent, a2a.NewTextPart(lastText))
	}
	return p.Print(task)
}

func sendSingle(ctx context.Context, p *output.Printer, t AgentTarget, msgText string) error {
	client, err := newClientForTarget(ctx, t)
	if err != nil {
		p.PrintError(err)
		return err
	}
	defer client.Destroy() //nolint:errcheck

	req, reqErr := buildSendRequest(msgText)
	if reqErr != nil {
		p.PrintError(reqErr)
		return reqErr
	}

	// lastText accumulates the most recent agent text seen across working events.
	// Some agents (e.g. Rover) deliver the final answer in the last working event
	// and send a bare completed event with no message. In human mode we print
	// lastText as the final answer; in json mode we always print the task.
	var lastText string

	for event, err := range client.SendStreamingMessage(ctx, req) {
		if err != nil {
			p.PrintError(fmt.Errorf("stream error: %w", err))
			return err
		}

		switch resp := event.(type) {
		case *a2a.Message:
			return p.Print(resp)
		case *a2a.Task:
			state := resp.Status.State
			if state.Terminal() || state == a2a.TaskStateInputRequired {
				if state == a2a.TaskStateInputRequired {
					return handleInputRequired(ctx, p, client, t, resp)
				}
				collectText(resp.Status.Message, &lastText)
				return printFinal(p, resp, lastText)
			}
			collectText(resp.Status.Message, &lastText)
		case *a2a.TaskStatusUpdateEvent:
			state := resp.Status.State
			if state == a2a.TaskStateAuthRequired {
				if handleAuthErr := handleAuthRequired(ctx, p, resp); handleAuthErr != nil {
					return handleAuthErr
				}
				continue
			}
			if state == a2a.TaskStateInputRequired {
				return fmt.Errorf("input required (task-id: %s)", resp.TaskID)
			}
			if state.Terminal() {
				collectText(resp.Status.Message, &lastText)
				fakeTask := &a2a.Task{
					ID:        resp.TaskID,
					ContextID: resp.ContextID,
					Status:    resp.Status,
				}
				return printFinal(p, fakeTask, lastText)
			}
			collectText(resp.Status.Message, &lastText)
		}
	}
	// Stream ended without a terminal event — print whatever answer we collected.
	if lastText != "" {
		p.PrintLine(lastText)
	}
	return nil
}

// handleInputRequired: prompt in terminal; exit 2 in json mode.
func handleInputRequired(
	ctx context.Context,
	p *output.Printer,
	client interface {
		SendMessage(context.Context, *a2a.SendMessageRequest) (a2a.SendMessageResult, error)
	},
	t AgentTarget,
	task *a2a.Task,
) error {
	if flags.format == "json" {
		p.Print(task) //nolint:errcheck
		return ErrInputRequired
	}

	// Human mode: show the question
	p.Print(task) //nolint:errcheck

	if task.ID == "" {
		return nil
	}

	if isTerminal() {
		fmt.Fprint(os.Stderr, "\nYour reply: ")
		scanner := bufio.NewScanner(os.Stdin)
		if scanner.Scan() {
			reply := strings.TrimSpace(scanner.Text())
			if reply == "" {
				return nil
			}
			if err := validate.MessageText(reply, "reply"); err != nil {
				p.PrintError(err)
				return err
			}
			msg := a2a.NewMessage(a2a.MessageRoleUser, a2a.NewTextPart(reply))
			msg.TaskID = task.ID
			msg.ContextID = task.ContextID
			result, err := client.SendMessage(ctx, &a2a.SendMessageRequest{Message: msg})
			if err != nil {
				p.PrintError(fmt.Errorf("reply failed: %w", err))
				return err
			}
			switch r := result.(type) {
			case *a2a.Task:
				return p.Print(r)
			case *a2a.Message:
				return p.Print(r)
			}
		}
	} else {
		fmt.Fprintf(os.Stderr, "\nTo reply: agc --agent %s send --task-id %s \"your response\"\n",
			t.Label(), task.ID)
	}
	return nil
}

// handleAuthRequired: open browser in terminal; exit 3 in json mode.
func handleAuthRequired(
	ctx context.Context,
	p *output.Printer,
	event *a2a.TaskStatusUpdateEvent,
) error {
	if flags.format == "json" {
		// Build a minimal representation for the AI agent
		type authReq struct {
			Type      string `json:"type"`
			TaskID    string `json:"taskId"`
			ContextID string `json:"contextId"`
			Message   string `json:"message,omitempty"`
		}
		msg := ""
		if event.Status.Message != nil {
			for _, p := range event.Status.Message.Parts {
				msg += p.Text()
			}
		}
		p.Print(authReq{ //nolint:errcheck
			Type: "TASK_STATE_AUTH_REQUIRED", TaskID: string(event.TaskID),
			ContextID: event.ContextID, Message: msg,
		})
		return ErrAuthRequired
	}

	fmt.Fprintln(os.Stderr, "\nAuthentication required")
	if event.Status.Message != nil {
		for _, part := range event.Status.Message.Parts {
			if txt := part.Text(); txt != "" {
				fmt.Fprintln(os.Stderr, txt)
				if url := extractURL(txt); url != "" {
					fmt.Fprintf(os.Stderr, "\nOpening browser: %s\n", url)
					openBrowserURL(url)
				}
			}
		}
	}
	fmt.Fprintln(os.Stderr, "\nWaiting for authentication...")
	fmt.Fprintf(os.Stderr, "(Check: agc task get --id %s)\n", event.TaskID)
	_ = ctx
	return nil // streaming loop continues
}

// --------------------------------------------------------------------------
// Multi-agent parallel
// --------------------------------------------------------------------------

func sendParallel(ctx context.Context, p *output.Printer, targets []AgentTarget, msgText string) error {
	type result struct {
		val any
		err bool
	}
	ch := make(chan result, len(targets))
	var wg sync.WaitGroup

	for _, t := range targets {
		wg.Add(1)
		go func(t AgentTarget) {
			defer wg.Done()
			client, err := newClientForTarget(ctx, t)
			if err != nil {
				ch <- result{val: map[string]any{"agent": t.Label(), "error": err.Error()}, err: true}
				return
			}
			defer client.Destroy() //nolint:errcheck

			req, reqErr := buildSendRequest(msgText)
			if reqErr != nil {
				ch <- result{val: map[string]any{"agent": t.Label(), "error": reqErr.Error()}, err: true}
				return
			}
			raw, sendErr := client.SendMessage(ctx, req)
			if sendErr != nil {
				ch <- result{val: map[string]any{"agent": t.Label(), "error": sendErr.Error()}, err: true}
				return
			}
			// Wrap raw result with agent label
			type agentResult struct {
				Agent    string `json:"agent"`
				AgentURL string `json:"agentUrl"`
				Result   any    `json:"result"`
			}
			ch <- result{val: agentResult{Agent: t.Label(), AgentURL: t.URL, Result: raw}}
		}(t)
	}
	go func() { wg.Wait(); close(ch) }()

	var anyError bool
	for r := range ch {
		if printErr := p.PrintJSON(r.val); printErr != nil {
			return printErr
		}
		if r.err {
			anyError = true
		}
	}
	if anyError {
		return fmt.Errorf("one or more agents returned errors")
	}
	return nil
}

// --------------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------------

func buildSendRequest(msgText string) (*a2a.SendMessageRequest, error) {
	if sendFlags.params != "" {
		if err := validate.DangerousInput(sendFlags.params, "--params"); err != nil {
			return nil, err
		}
		var req a2a.SendMessageRequest
		if err := json.Unmarshal([]byte(sendFlags.params), &req); err != nil {
			return nil, fmt.Errorf("--params: invalid SendMessageRequest JSON: %w\nSee: agc schema send", err)
		}
		if req.Message != nil {
			if req.Message.ID == "" {
				req.Message.ID = a2a.NewMessageID()
			}
			if req.Message.Role == a2a.MessageRoleUnspecified {
				req.Message.Role = a2a.MessageRoleUser
			}
		}
		return &req, nil
	}
	msg := a2a.NewMessage(a2a.MessageRoleUser, a2a.NewTextPart(msgText))
	if sendFlags.taskID != "" {
		msg.TaskID = a2a.TaskID(sendFlags.taskID)
	}
	if sendFlags.contextID != "" {
		msg.ContextID = sendFlags.contextID
	}
	req := &a2a.SendMessageRequest{Message: msg}
	if sendFlags.historyLength > 0 {
		req.Config = &a2a.SendMessageConfig{HistoryLength: &sendFlags.historyLength}
	}
	return req, nil
}
func resolveMessage(args []string, stdin io.Reader) (string, error) {
	if len(args) > 0 && args[0] != "" {
		return args[0], nil
	}
	data, err := io.ReadAll(stdin)
	if err != nil {
		return "", fmt.Errorf("read stdin: %w", err)
	}
	msg := strings.TrimSpace(string(data))
	if msg == "" {
		return "", fmt.Errorf("message is required: provide as argument or via stdin")
	}
	return msg, nil
}

func openBrowserURL(rawURL string) {
	var cmd string
	var args []string
	switch runtime.GOOS {
	case "darwin":
		cmd, args = "open", []string{rawURL}
	case "windows":
		cmd, args = "cmd", []string{"/c", "start", rawURL}
	default:
		cmd, args = "xdg-open", []string{rawURL}
	}
	_ = exec.Command(cmd, args...).Start()
}

func extractURL(text string) string {
	for _, word := range strings.Fields(text) {
		if strings.HasPrefix(word, "http://") || strings.HasPrefix(word, "https://") {
			return strings.TrimRight(word, ".,;:)\n")
		}
	}
	return ""
}
