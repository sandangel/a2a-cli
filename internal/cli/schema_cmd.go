package cli

import (
	"encoding/json"
	"fmt"
	"sort"
	"strings"

	"github.com/spf13/cobra"
)

var schemaCmd = &cobra.Command{
	Use:   "schema [type]",
	Short: "Show A2A data structure schemas — inspect before you call",
	Long: `Show JSON schemas for A2A protocol types and operations.

Learn the data structures before making API calls.

PROTOCOL TYPES
  agc schema                     List all operations with their input/output types
  agc schema send                SendMessageRequest — what to pass to agc send
  agc schema task                Task — what task operations return
  agc schema message             Message — the core message type
  agc schema part                Part — text, data, url, or raw content
  agc schema card                AgentCard — what agc card returns
  agc schema skill               AgentSkill — skill definition in the card
  agc schema artifact            Artifact — task output

FLAGS
  --resolve-refs   Inline all $ref references for a complete flat schema`,
	Example: `  # Learn the schema before sending
  agc schema send
  agc schema task

  # Resolved schema with all refs inlined
  agc schema task --resolve-refs`,
	Args: cobra.MaximumNArgs(1),
	RunE: runSchema,
}

var schemaResolveRefs bool

func init() {
	schemaCmd.Flags().BoolVar(&schemaResolveRefs, "resolve-refs", false,
		"Inline all $ref references into a complete flat schema")
	rootCmd.AddCommand(schemaCmd)
}

func runSchema(cmd *cobra.Command, args []string) error {
	p := printer()

	if len(args) == 0 {
		return p.Print(allOperations())
	}

	path := args[0]
	schema, ok := a2aSchemas[path]
	if !ok {
		known := make([]string, 0, len(a2aSchemas))
		for k := range a2aSchemas {
			known = append(known, k)
		}
		sort.Strings(known)
		return fmt.Errorf("unknown type %q\nBuilt-in types: %s", path, strings.Join(known, ", "))
	}

	out := schema
	if schemaResolveRefs {
		out = resolveRefs(schema, a2aSchemas)
	}
	raw, _ := json.MarshalIndent(out, "", "  ")
	fmt.Println(string(raw))
	return nil
}

func resolveRefs(schema map[string]any, all map[string]map[string]any) map[string]any {
	out := make(map[string]any, len(schema))
	for k, v := range schema {
		switch val := v.(type) {
		case map[string]any:
			if ref, ok := val["$ref"].(string); ok {
				if target, found := all[ref]; found {
					out[k] = resolveRefs(target, all)
				} else {
					out[k] = val
				}
			} else {
				out[k] = resolveRefs(val, all)
			}
		default:
			out[k] = v
		}
	}
	return out
}

func allOperations() any {
	return map[string]any{
		"description": "A2A protocol operations. Use 'agc schema <type>' to inspect any data structure.",
		"operations": []map[string]any{
			{
				"command":      "agc send [text] [--params '{...}']",
				"operation":    "SendMessage",
				"schema":       "agc schema send",
				"returns":      "AgentResponse",
				"returnSchema": "agc schema task  OR  agc schema message",
			},
			{
				"command":      "agc task get --id <id>",
				"operation":    "GetTask",
				"returns":      "AgentResponse",
				"returnSchema": "agc schema task",
			},
			{
				"command":      "agc task list",
				"operation":    "ListTasks",
				"returns":      "AgentResponse (array of Task)",
				"returnSchema": "agc schema task",
			},
			{
				"command":      "agc task cancel --id <id>",
				"operation":    "CancelTask",
				"returns":      "AgentResponse",
				"returnSchema": "agc schema task",
			},
			{
				"command":      "agc card",
				"operation":    "GetAgentCard",
				"returns":      "AgentCard",
				"returnSchema": "agc schema card",
			},
		},
		"builtinSchemas": []string{"send", "task", "message", "part", "card", "skill", "artifact"},
		"tip":            "Use --format json to get machine-readable AgentResponse output. Always read .text for the answer.",
	}
}

// a2aSchemas — JSON Schema objects for all core A2A types, derived from a2a.proto.
var a2aSchemas = map[string]map[string]any{
	"send": {
		"description": "SendMessageRequest — the input to 'agc send'. Pass the full structure via --params or just provide text as an argument.",
		"type":        "object",
		"required":    []string{"message"},
		"properties": map[string]any{
			"message": map[string]any{
				"$ref":        "message",
				"description": "Required. The message to send.",
			},
			"configuration": map[string]any{
				"type": "object",
				"properties": map[string]any{
					"historyLength":     map[string]any{"type": "integer", "description": "Messages from history to include in response."},
					"returnImmediately": map[string]any{"type": "boolean", "description": "Return without waiting for task completion (default false)."},
				},
			},
			"metadata": map[string]any{"type": "object", "description": "Extension metadata."},
		},
		"returns": "AgentResponse",
		"examples": []map[string]any{
			{
				"_comment": "Simple text message",
				"message":  map[string]any{"parts": []map[string]any{{"text": "your request here"}}},
			},
			{
				"_comment": "Continue a task",
				"message":  map[string]any{"parts": []map[string]any{{"text": "follow up"}}, "taskId": "task-uuid", "contextId": "ctx-uuid"},
			},
		},
		"cliUsage": []string{
			`agc --format json send "simple text"`,
			`agc --format json send --params '{"message":{"parts":[{"text":"your request"}],"taskId":"task-uuid"}}'`,
		},
	},

	"message": {
		"description": "Message — a conversation turn between user and agent.",
		"type":        "object",
		"required":    []string{"messageId", "role", "parts"},
		"properties": map[string]any{
			"messageId":        map[string]any{"type": "string", "description": "Client-generated unique ID (UUID recommended)."},
			"role":             map[string]any{"type": "string", "enum": []string{"ROLE_USER", "ROLE_AGENT"}, "description": "Sender."},
			"parts":            map[string]any{"type": "array", "items": map[string]any{"$ref": "part"}, "description": "Content (one or more parts)."},
			"taskId":           map[string]any{"type": "string", "description": "Continue an existing task."},
			"contextId":        map[string]any{"type": "string", "description": "Conversation thread ID."},
			"extensions":       map[string]any{"type": "array", "items": map[string]any{"type": "string"}, "description": "Activated extension URIs."},
			"metadata":         map[string]any{"type": "object", "description": "Extension data."},
			"referenceTaskIds": map[string]any{"type": "array", "items": map[string]any{"type": "string"}, "description": "Related task IDs for context."},
		},
	},

	"part": {
		"description": "Part — one piece of content in a message or artifact. Set exactly one of: text, data, url, raw.",
		"type":        "object",
		"oneOf": []map[string]any{
			{"required": []string{"text"}, "properties": map[string]any{"text": map[string]any{"type": "string"}}},
			{"required": []string{"data"}, "properties": map[string]any{"data": map[string]any{"type": "object", "description": "Structured JSON."}}},
			{"required": []string{"url"}, "properties": map[string]any{"url": map[string]any{"type": "string", "format": "uri"}}},
			{"required": []string{"raw"}, "properties": map[string]any{"raw": map[string]any{"type": "string", "format": "base64"}}},
		},
		"properties": map[string]any{
			"mediaType": map[string]any{"type": "string", "description": "MIME type (e.g. 'text/plain', 'image/png', 'application/json')."},
			"filename":  map[string]any{"type": "string", "description": "Filename hint."},
			"metadata":  map[string]any{"type": "object"},
		},
	},

	"task": {
		"description": "Task — stateful unit of work with a lifecycle. Returned by send, task get, task list, task cancel.",
		"type":        "object",
		"properties": map[string]any{
			"id":        map[string]any{"type": "string", "description": "Server-generated task ID. Use with --task-id to continue."},
			"contextId": map[string]any{"type": "string", "description": "Conversation thread. Use with --context-id to start a new task in the same thread."},
			"status": map[string]any{
				"type": "object",
				"properties": map[string]any{
					"state": map[string]any{
						"type": "string",
						"enum": []string{
							"TASK_STATE_SUBMITTED",
							"TASK_STATE_WORKING",
							"TASK_STATE_COMPLETED",
							"TASK_STATE_FAILED",
							"TASK_STATE_CANCELED",
							"TASK_STATE_REJECTED",
							"TASK_STATE_INPUT_REQUIRED",
							"TASK_STATE_AUTH_REQUIRED",
						},
						"description": "Lifecycle state. COMPLETED/FAILED/CANCELED/REJECTED are terminal. INPUT_REQUIRED: reply via --task-id. AUTH_REQUIRED: authenticate out-of-band then poll.",
					},
					"message":   map[string]any{"$ref": "message", "description": "Agent message for this status."},
					"timestamp": map[string]any{"type": "string", "format": "date-time"},
				},
			},
			"artifacts": map[string]any{"type": "array", "items": map[string]any{"$ref": "artifact"}, "description": "Task outputs."},
			"history":   map[string]any{"type": "array", "items": map[string]any{"$ref": "message"}, "description": "Conversation history for this task."},
			"metadata":  map[string]any{"type": "object"},
		},
		"stateMachine": "SUBMITTED → WORKING → COMPLETED | FAILED | CANCELED | REJECTED | INPUT_REQUIRED | AUTH_REQUIRED",
	},

	"artifact": {
		"description": "Artifact — output produced by the agent (files, reports, generated content).",
		"type":        "object",
		"properties": map[string]any{
			"artifactId":  map[string]any{"type": "string"},
			"name":        map[string]any{"type": "string", "description": "Human-readable name."},
			"description": map[string]any{"type": "string"},
			"parts":       map[string]any{"type": "array", "items": map[string]any{"$ref": "part"}, "description": "Artifact content."},
			"metadata":    map[string]any{"type": "object"},
		},
	},

	"card": {
		"description": "AgentCard — agent identity, capabilities, skills, auth requirements. Fetched from /.well-known/agent-card.json.",
		"type":        "object",
		"properties": map[string]any{
			"name":        map[string]any{"type": "string"},
			"description": map[string]any{"type": "string"},
			"version":     map[string]any{"type": "string"},
			"capabilities": map[string]any{
				"type": "object",
				"properties": map[string]any{
					"streaming":         map[string]any{"type": "boolean"},
					"pushNotifications": map[string]any{"type": "boolean"},
					"extendedAgentCard": map[string]any{"type": "boolean", "description": "If true: agc auth login fetches richer card at /extendedAgentCard."},
					"extensions":        map[string]any{"type": "array", "description": "Supported extension URIs."},
				},
			},
			"skills":               map[string]any{"type": "array", "items": map[string]any{"$ref": "skill"}},
			"securitySchemes":      map[string]any{"type": "object", "description": "Auth schemes (OAuth2, Bearer, APIKey, OIDC, mTLS)."},
			"securityRequirements": map[string]any{"type": "array", "description": "Which schemes are required."},
			"supportedInterfaces":  map[string]any{"type": "array", "description": "Transport endpoints (JSONRPC, HTTP+JSON)."},
		},
	},

	"skill": {
		"description": "AgentSkill — a capability declared in the agent card.",
		"type":        "object",
		"properties": map[string]any{
			"id":          map[string]any{"type": "string"},
			"name":        map[string]any{"type": "string"},
			"description": map[string]any{"type": "string"},
			"tags":        map[string]any{"type": "array", "items": map[string]any{"type": "string"}},
			"examples":    map[string]any{"type": "array", "items": map[string]any{"type": "string"}, "description": "Example prompts."},
			"inputModes":  map[string]any{"type": "array", "items": map[string]any{"type": "string"}, "description": "MIME types accepted."},
			"outputModes": map[string]any{"type": "array", "items": map[string]any{"type": "string"}, "description": "MIME types produced."},
		},
	},
}
