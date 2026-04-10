package cli

import (
	"context"
	"fmt"
	"sync"
	"time"

	"github.com/a2aproject/a2a-go/v2/a2a"
	"github.com/spf13/cobra"
	"genai.stargate.toyota/agc/internal/output"
	"genai.stargate.toyota/agc/internal/validate"
)

var taskCmd = &cobra.Command{
	Use:   "task",
	Short: "Manage tasks — get, list, cancel",
}

// ---- task get ----

var taskGetFlags struct {
	id            string
	historyLength int
}

var taskGetCmd = &cobra.Command{
	Use:   "get",
	Short: "Get a task by ID",
	Example: `  agc task get --id task-abc
  agc --agent prod task get --id task-abc
  agc --json task get --id task-abc | jq .text`,
	RunE: runTaskGet,
}

func init() {
	taskGetCmd.Flags().StringVar(&taskGetFlags.id, "id", "", "Task ID (required)")
	taskGetCmd.Flags().IntVar(&taskGetFlags.historyLength, "history-length", 0, "History messages to include")
	_ = taskGetCmd.MarkFlagRequired("id")
	taskCmd.AddCommand(taskGetCmd)
	taskCmd.AddCommand(taskListCmd)
	taskCmd.AddCommand(taskCancelCmd)
}

func runTaskGet(cmd *cobra.Command, _ []string) error {
	p := printer()

	if err := validate.Identifier(taskGetFlags.id, "--id"); err != nil {
		p.PrintError(err)
		return err
	}

	targets, err := resolveTargets()
	if err != nil {
		p.PrintError(err)
		return err
	}
	if len(targets) > 1 {
		err := fmt.Errorf("task get requires a single agent (task IDs are agent-specific)")
		p.PrintError(err)
		return err
	}
	t := targets[0]

	ctx, cancel := cmdTimeout(cmd)
	defer cancel()

	client, err := newClientForTarget(ctx, t)
	if err != nil {
		p.PrintError(err)
		return err
	}
	defer client.Destroy() //nolint:errcheck

	req := &a2a.GetTaskRequest{ID: a2a.TaskID(taskGetFlags.id)}
	if taskGetFlags.historyLength > 0 {
		req.HistoryLength = &taskGetFlags.historyLength
	}
	task, err := client.GetTask(ctx, req)
	if err != nil {
		p.PrintError(fmt.Errorf("get task: %w", err))
		return err
	}

	if flags.format == "json" {
		return p.Print(task)
	}
	resp := output.NormalizeTask(t.Label(), t.URL, task)
	return p.PrintResponse(resp, false)
}

// ---- task list ----

var taskListFlags struct {
	contextID        string
	status           string
	pageSize         int
	pageToken        string
	includeArtifacts bool
}

var taskListCmd = &cobra.Command{
	Use:   "list",
	Short: "List tasks — use --all to list across all agents",
	Example: `  agc task list
  agc --all task list
  agc task list --status TASK_STATE_WORKING
  agc --json --all task list | jq '{agent, total: .total_size}'`,
	RunE: runTaskList,
}

func init() {
	taskListCmd.Flags().StringVar(&taskListFlags.contextID, "context-id", "", "Filter by context ID")
	taskListCmd.Flags().StringVar(&taskListFlags.status, "status", "", "Filter by state")
	taskListCmd.Flags().IntVar(&taskListFlags.pageSize, "page-size", 20, "Max tasks per agent")
	taskListCmd.Flags().StringVar(&taskListFlags.pageToken, "page-token", "", "Pagination token")
	taskListCmd.Flags().BoolVar(&taskListFlags.includeArtifacts, "include-artifacts", false, "Include artifacts")
}

func runTaskList(cmd *cobra.Command, _ []string) error {
	p := printer()

	if taskListFlags.contextID != "" {
		if err := validate.Identifier(taskListFlags.contextID, "--context-id"); err != nil {
			p.PrintError(err)
			return err
		}
	}

	targets, err := resolveTargets()
	if err != nil {
		p.PrintError(err)
		return err
	}

	ctx, cancel := cmdTimeout(cmd)
	defer cancel()

	if len(targets) == 1 {
		return taskListForTarget(ctx, p, targets[0], false)
	}

	type result struct {
		target AgentTarget
		err    error
	}
	ch := make(chan result, len(targets))
	var wg sync.WaitGroup
	for _, t := range targets {
		wg.Add(1)
		go func(t AgentTarget) {
			defer wg.Done()
			ch <- result{target: t, err: taskListForTarget(ctx, p, t, true)}
		}(t)
	}
	go func() { wg.Wait(); close(ch) }()

	var anyError bool
	for r := range ch {
		if r.err != nil {
			anyError = true
		}
	}
	if anyError {
		return fmt.Errorf("one or more agents returned errors")
	}
	return nil
}

func taskListForTarget(ctx context.Context, p *output.Printer, t AgentTarget, multiAgent bool) error {
	client, err := newClientForTarget(ctx, t)
	if err != nil {
		if multiAgent {
			p.PrintError(fmt.Errorf("[%s] %v", t.Label(), err))
			return err
		}
		p.PrintError(err)
		return err
	}
	defer client.Destroy() //nolint:errcheck

	req := &a2a.ListTasksRequest{
		ContextID:        taskListFlags.contextID,
		PageSize:         taskListFlags.pageSize,
		PageToken:        taskListFlags.pageToken,
		IncludeArtifacts: taskListFlags.includeArtifacts,
	}
	if taskListFlags.status != "" {
		req.Status = a2a.TaskState(taskListFlags.status)
	}

	resp, err := client.ListTasks(ctx, req)
	if err != nil {
		p.PrintError(fmt.Errorf("[%s] list tasks: %w", t.Label(), err))
		return err
	}

	if flags.format == "json" {
		type taskListJSON struct {
			Agent         string       `json:"agent"`
			AgentURL      string       `json:"agent_url"`
			TotalSize     int          `json:"total_size"`
			NextPageToken string       `json:"next_page_token,omitempty"`
			Tasks         []*a2a.Task  `json:"tasks"`
		}
		v := taskListJSON{
			Agent:         t.Label(),
			AgentURL:      t.URL,
			TotalSize:     resp.TotalSize,
			NextPageToken: resp.NextPageToken,
			Tasks:         resp.Tasks,
		}
		if multiAgent {
			return p.PrintJSON(v)
		}
		return p.Print(v)
	}

	// Human output.
	if multiAgent {
		p.PrintLine(fmt.Sprintf("── %s ── %d task(s)", t.Label(), resp.TotalSize))
	} else {
		p.PrintLine(fmt.Sprintf("%d task(s) for %s", resp.TotalSize, t.Label()))
	}
	if len(resp.Tasks) == 0 {
		p.PrintLine("  (none)")
		return nil
	}
	headers := []string{"TASK ID", "STATE", "UPDATED", "SUMMARY"}
	var rows [][]string
	for _, task := range resp.Tasks {
		updated := ""
		if task.Status.Timestamp != nil {
			updated = task.Status.Timestamp.Format(time.DateTime)
		}
		summary := ""
		if task.Status.Message != nil {
			for _, part := range task.Status.Message.Parts {
				if t := part.Text(); t != "" {
					summary = output.Truncate(t, 50)
					break
				}
			}
		}
		rows = append(rows, []string{string(task.ID), string(task.Status.State), updated, summary})
	}
	p.PrintTable(headers, rows)
	if resp.NextPageToken != "" {
		p.PrintLine(fmt.Sprintf("\nMore results — next page: --page-token %s", resp.NextPageToken))
	}
	return nil
}

// ---- task cancel ----

var taskCancelFlags struct{ id string }

var taskCancelCmd = &cobra.Command{
	Use:   "cancel",
	Short: "Cancel a task by ID",
	Example: `  agc task cancel --id task-abc
  agc --agent prod task cancel --id task-abc`,
	RunE: runTaskCancel,
}

func init() {
	taskCancelCmd.Flags().StringVar(&taskCancelFlags.id, "id", "", "Task ID (required)")
	_ = taskCancelCmd.MarkFlagRequired("id")
}

func runTaskCancel(cmd *cobra.Command, _ []string) error {
	p := printer()

	if err := validate.Identifier(taskCancelFlags.id, "--id"); err != nil {
		p.PrintError(err)
		return err
	}

	targets, err := resolveTargets()
	if err != nil {
		p.PrintError(err)
		return err
	}
	if len(targets) > 1 {
		err := fmt.Errorf("task cancel requires a single agent")
		p.PrintError(err)
		return err
	}
	t := targets[0]

	ctx, cancel := cmdTimeout(cmd)
	defer cancel()

	client, err := newClientForTarget(ctx, t)
	if err != nil {
		p.PrintError(err)
		return err
	}
	defer client.Destroy() //nolint:errcheck

	task, err := client.CancelTask(ctx, &a2a.CancelTaskRequest{ID: a2a.TaskID(taskCancelFlags.id)})
	if err != nil {
		p.PrintError(fmt.Errorf("cancel task: %w", err))
		return err
	}

	if flags.format == "json" {
		return p.Print(task)
	}
	resp := output.NormalizeTask(t.Label(), t.URL, task)
	return p.PrintResponse(resp, false)
}
