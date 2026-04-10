package output

import (
	"strings"
)

// FilterFields applies a comma-separated dotted-path field mask to a JSON-decoded
// value (map[string]any, []any, or scalar), returning only the requested paths.
//
// Syntax mirrors what you'd pass to jq or Google API field masks:
//   "id,status.state,status.message"         → nested dotted paths
//   "artifacts,history"                       → whole sub-trees
//   "skills.id,skills.name"                   → repeated array sub-fields
//
// Example:
//   input:  {"id":"t1","contextId":"c1","status":{"state":"COMPLETED","message":{...}},"history":[...]}
//   fields: "id,status.state"
//   output: {"id":"t1","status":{"state":"COMPLETED"}}
func FilterFields(v any, fields string) any {
	if fields == "" {
		return v
	}
	paths := parsePaths(fields)
	if len(paths) == 0 {
		return v
	}
	return applyPaths(v, paths)
}

// parsePaths splits "a,b.c,d.e.f" into [["a"],["b","c"],["d","e","f"]].
func parsePaths(fields string) [][]string {
	var result [][]string
	for _, raw := range strings.Split(fields, ",") {
		raw = strings.TrimSpace(raw)
		if raw == "" {
			continue
		}
		result = append(result, strings.Split(raw, "."))
	}
	return result
}

// applyPaths recursively projects only the requested paths from v.
func applyPaths(v any, paths [][]string) any {
	switch val := v.(type) {
	case map[string]any:
		return projectMap(val, paths)
	case []any:
		return projectSlice(val, paths)
	default:
		return v
	}
}

// projectMap extracts only the fields named by paths from a map.
func projectMap(m map[string]any, paths [][]string) map[string]any {
	out := make(map[string]any)

	// Group paths by their first segment.
	byFirst := make(map[string][][]string)
	for _, path := range paths {
		if len(path) == 0 {
			continue
		}
		head := path[0]
		rest := path[1:]
		byFirst[head] = append(byFirst[head], rest)
	}

	for head, tails := range byFirst {
		val, ok := m[head]
		if !ok {
			continue
		}

		// Are any tails non-empty? If so we need to recurse.
		hasSubPaths := false
		for _, t := range tails {
			if len(t) > 0 {
				hasSubPaths = true
				break
			}
		}

		if !hasSubPaths {
			// No sub-paths — include the entire value.
			out[head] = val
		} else {
			// Recurse with the remaining sub-paths.
			out[head] = applyPaths(val, tails)
		}
	}
	return out
}

// projectSlice applies the same path projection to every element of a slice.
func projectSlice(arr []any, paths [][]string) []any {
	out := make([]any, len(arr))
	for i, elem := range arr {
		out[i] = applyPaths(elem, paths)
	}
	return out
}
