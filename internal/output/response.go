package output

import (
	"strings"

	"github.com/a2aproject/a2a-go/v2/a2a"
)

// AgentResponse is the normalized shape for every agc response.
// The `agent` and `agent_url` fields always identify the source agent.
// The `text` field is always the primary content — use it to read the answer.
type AgentResponse struct {
	Agent    string `json:"agent"`              // alias name (or URL if no alias)
	AgentURL string `json:"agent_url"`          // agent base URL
	Type     string `json:"type"`               // message | task_completed | task_failed | task_working | input_required | artifact_update | error
	Text     string `json:"text,omitempty"`     // extracted plaintext — the primary answer field
	Role     string `json:"role,omitempty"`     // ROLE_AGENT | ROLE_USER (for message type)
	State    string `json:"state,omitempty"`    // task state (for task types)
	TaskID   string `json:"task_id,omitempty"`
	ContextID string `json:"context_id,omitempty"`
	MessageID string `json:"message_id,omitempty"`
	Artifacts []AgentArtifact `json:"artifacts,omitempty"`
	DataParts []any           `json:"data,omitempty"`
	InputRequired bool        `json:"input_required,omitempty"`
	Error    string         `json:"error,omitempty"`    // set when type=error
	Metadata map[string]any `json:"metadata,omitempty"` // extension metadata (e.g. skill_id)
}

// AgentArtifact is a normalized artifact.
type AgentArtifact struct {
	ID        string `json:"id"`
	Name      string `json:"name,omitempty"`
	Text      string `json:"text,omitempty"`
	MediaType string `json:"media_type,omitempty"`
	DataParts []any  `json:"data,omitempty"`
}

// ErrorResponse returns an AgentResponse representing a client-side error
// (e.g. auth failure, network error).
func ErrorResponse(agent, agentURL, errMsg string) AgentResponse {
	return AgentResponse{
		Agent:    agent,
		AgentURL: agentURL,
		Type:     "error",
		Error:    errMsg,
	}
}

// NormalizeSendResult converts a SendMessageResult to an AgentResponse.
func NormalizeSendResult(agent, agentURL string, result a2a.SendMessageResult) AgentResponse {
	switch v := result.(type) {
	case *a2a.Message:
		return normalizeMessage(agent, agentURL, v)
	case *a2a.Task:
		return normalizeTask(agent, agentURL, v)
	default:
		return AgentResponse{Agent: agent, AgentURL: agentURL, Type: "unknown"}
	}
}

// NormalizeEvent converts a streaming Event to an AgentResponse.
func NormalizeEvent(agent, agentURL string, event a2a.Event) AgentResponse {
	switch e := event.(type) {
	case *a2a.Message:
		return normalizeMessage(agent, agentURL, e)
	case *a2a.Task:
		return normalizeTask(agent, agentURL, e)
	case *a2a.TaskStatusUpdateEvent:
		return normalizeStatusUpdate(agent, agentURL, e)
	case *a2a.TaskArtifactUpdateEvent:
		return normalizeArtifactUpdate(agent, agentURL, e)
	default:
		return AgentResponse{Agent: agent, AgentURL: agentURL, Type: "unknown"}
	}
}

func normalizeMessage(agent, agentURL string, m *a2a.Message) AgentResponse {
	r := AgentResponse{
		Agent:     agent,
		AgentURL:  agentURL,
		Type:      "message",
		Role:      string(m.Role),
		TaskID:    string(m.TaskID),
		ContextID: m.ContextID,
		MessageID: m.ID,
	}
	r.Text, r.DataParts = extractParts(m.Parts)
	r.Text = sanitizeForTerminal(r.Text)
	return r
}

func normalizeTask(agent, agentURL string, t *a2a.Task) AgentResponse {
	r := AgentResponse{
		Agent:     agent,
		AgentURL:  agentURL,
		Type:      taskResponseType(t.Status.State),
		State:     string(t.Status.State),
		TaskID:    string(t.ID),
		ContextID: t.ContextID,
		InputRequired: t.Status.State == a2a.TaskStateInputRequired ||
			t.Status.State == a2a.TaskStateAuthRequired,
	}
	if t.Status.Message != nil {
		r.MessageID = t.Status.Message.ID
		r.Text, r.DataParts = extractParts(t.Status.Message.Parts)
		r.Text = sanitizeForTerminal(r.Text)
	}
	for _, art := range t.Artifacts {
		r.Artifacts = append(r.Artifacts, normalizeArtifact(art))
	}
	return r
}

func normalizeStatusUpdate(agent, agentURL string, e *a2a.TaskStatusUpdateEvent) AgentResponse {
	r := AgentResponse{
		Agent:     agent,
		AgentURL:  agentURL,
		Type:      taskResponseType(e.Status.State),
		State:     string(e.Status.State),
		TaskID:    string(e.TaskID),
		ContextID: e.ContextID,
		InputRequired: e.Status.State == a2a.TaskStateInputRequired ||
			e.Status.State == a2a.TaskStateAuthRequired,
	}
	if e.Status.Message != nil {
		r.MessageID = e.Status.Message.ID
		r.Text, r.DataParts = extractParts(e.Status.Message.Parts)
		r.Text = sanitizeForTerminal(r.Text)
	}
	return r
}

func normalizeArtifactUpdate(agent, agentURL string, e *a2a.TaskArtifactUpdateEvent) AgentResponse {
	r := AgentResponse{
		Agent:     agent,
		AgentURL:  agentURL,
		Type:      "artifact_update",
		TaskID:    string(e.TaskID),
		ContextID: e.ContextID,
	}
	if e.Artifact != nil {
		art := normalizeArtifact(e.Artifact)
		r.Artifacts = append(r.Artifacts, art)
		r.Text = art.Text
	}
	return r
}

func taskResponseType(state a2a.TaskState) string {
	switch state {
	case a2a.TaskStateCompleted:
		return "task_completed"
	case a2a.TaskStateFailed:
		return "task_failed"
	case a2a.TaskStateCanceled:
		return "task_canceled"
	case a2a.TaskStateRejected:
		return "task_rejected"
	case a2a.TaskStateWorking:
		return "task_working"
	case a2a.TaskStateSubmitted:
		return "task_submitted"
	case a2a.TaskStateInputRequired:
		return "input_required"
	case a2a.TaskStateAuthRequired:
		return "auth_required"
	default:
		return "task"
	}
}

func extractParts(parts a2a.ContentParts) (text string, data []any) {
	var textParts []string
	for _, p := range parts {
		if p == nil {
			continue
		}
		if t := p.Text(); t != "" {
			textParts = append(textParts, t)
		}
		if d := p.Data(); d != nil {
			data = append(data, d)
		}
	}
	return strings.Join(textParts, "\n"), data
}

func normalizeArtifact(art *a2a.Artifact) AgentArtifact {
	na := AgentArtifact{ID: string(art.ID), Name: art.Name}
	na.Text, na.DataParts = extractParts(art.Parts)
	na.Text = sanitizeForTerminal(na.Text)
	for _, p := range art.Parts {
		if p != nil && p.MediaType != "" {
			na.MediaType = p.MediaType
			break
		}
	}
	return na
}

// NormalizeTask is the exported form for use by task commands.
func NormalizeTask(agent, agentURL string, t *a2a.Task) AgentResponse {
	return normalizeTask(agent, agentURL, t)
}
