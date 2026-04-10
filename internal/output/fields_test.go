package output_test

import (
	"encoding/json"
	"testing"

	"genai.stargate.toyota/agc/internal/output"
)

func TestFilterFields_TopLevel(t *testing.T) {
	input := map[string]any{"id": "t1", "contextId": "c1", "status": map[string]any{"state": "DONE"}}
	got := output.FilterFields(input, "id")
	m := got.(map[string]any)
	if m["id"] != "t1" {
		t.Errorf("expected id=t1, got %v", m["id"])
	}
	if _, ok := m["contextId"]; ok {
		t.Error("contextId should be filtered out")
	}
}

func TestFilterFields_Nested(t *testing.T) {
	input := map[string]any{
		"id":     "t1",
		"status": map[string]any{"state": "DONE", "message": map[string]any{"parts": []any{}}},
	}
	got := output.FilterFields(input, "id,status.state").(map[string]any)
	if got["id"] != "t1" {
		t.Errorf("id missing: %v", got)
	}
	status := got["status"].(map[string]any)
	if status["state"] != "DONE" {
		t.Errorf("status.state missing: %v", status)
	}
	if _, ok := status["message"]; ok {
		t.Error("status.message should be filtered out")
	}
}

func TestFilterFields_Array(t *testing.T) {
	input := []any{
		map[string]any{"id": "1", "name": "a", "extra": "x"},
		map[string]any{"id": "2", "name": "b", "extra": "y"},
	}
	got := output.FilterFields(input, "id,name").([]any)
	if len(got) != 2 {
		t.Fatalf("expected 2 elements, got %d", len(got))
	}
	first := got[0].(map[string]any)
	if first["id"] != "1" || first["name"] != "a" {
		t.Errorf("wrong first element: %v", first)
	}
	if _, ok := first["extra"]; ok {
		t.Error("extra should be filtered out")
	}
}

func TestFilterFields_Empty(t *testing.T) {
	input := map[string]any{"id": "t1"}
	got := output.FilterFields(input, "")
	if _, ok := got.(map[string]any); !ok {
		t.Error("empty fields should return same type unchanged")
	}
}

func TestFilterFields_DeepNested(t *testing.T) {
	raw := `{"id":"t1","status":{"state":"DONE","message":{"parts":[{"text":"Hello"}]}},"artifacts":[]}`
	var input any
	json.Unmarshal([]byte(raw), &input)

	got := output.FilterFields(input, "id,status.message.parts")
	b, _ := json.Marshal(got)
	var result map[string]any
	json.Unmarshal(b, &result)

	if result["id"] != "t1" {
		t.Errorf("id missing: %v", result)
	}
	if _, ok := result["artifacts"]; ok {
		t.Error("artifacts should be filtered out")
	}
	status := result["status"].(map[string]any)
	if _, ok := status["state"]; ok {
		t.Error("status.state should be filtered out when only status.message.parts is requested")
	}
	msg := status["message"].(map[string]any)
	parts := msg["parts"].([]any)
	if len(parts) != 1 {
		t.Errorf("expected 1 part, got %d", len(parts))
	}
}
