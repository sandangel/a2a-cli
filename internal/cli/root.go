// Package cli implements the agc command-line interface.
package cli

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/signal"
	"strings"
	"syscall"
	"time"

	"github.com/a2aproject/a2a-go/v2/a2a"
	"github.com/a2aproject/a2a-go/v2/a2aclient"
	"github.com/a2aproject/a2a-go/v2/a2aclient/agentcard"
	"github.com/a2aproject/a2a-go/v2/a2acompat/a2av0"
	"github.com/spf13/cobra"
	"genai.stargate.toyota/agc/internal/auth"
	"genai.stargate.toyota/agc/internal/config"
	"genai.stargate.toyota/agc/internal/output"
	"genai.stargate.toyota/agc/internal/validate"
)

var version = "0.1.0" // overridden at build time via -ldflags

type globalFlags struct {
	agents      []string      // --agent (repeatable): alias names or raw URLs
	allAgents   bool          // --all: every registered agent
	format      string
	fields      string  // --fields "a,b.c" — filter output fields
	transport   string  // --transport JSONRPC|GRPC|HTTP+JSON — override auto-negotiation
	
	timeout     time.Duration // --timeout
	bearerToken string        // --bearer-token
}

var flags globalFlags

var rootCmd = &cobra.Command{
	Use:   "agc",
	Short: "agc — Agent CLI for A2A protocol agents",
	Long: `agc (Agent CLI) — interact with A2A protocol agents.

By default output is human-friendly text — the agent's answer, plainly printed.
Use --format json for machine-readable output (AI tools, scripts).

Single agent (active or specified):
  agc send "Summarise the report"
  agc --agent prod send "Status?"
  agc --agent https://host send "Hello"

Multiple agents in parallel:
  agc --agent prod --agent staging send "Health check?"
  agc --all send "Status?"

Output modes:
  (default)  Human-friendly text — just the answer
  --format json   Raw A2A protocol JSON (Task, Message, AgentCard, etc.)
  --format table  Human-friendly table (default in terminal)

Register agents:
  agc agent add prod https://agent.example.com
  agc agent use prod

Authenticate:
  agc auth login`,
	Version:      version,
	SilenceUsage: true, // don't dump usage text on every runtime error
}

// Execute runs the root command, exiting with code 1 on error.
// Use ExecuteErr when you need to inspect the specific error for exit codes.
func Execute() {
	if err := ExecuteErr(); err != nil {
		os.Exit(1)
	}
}

// ExecuteErr runs the root command and returns the error for the caller to handle.
// It installs a signal handler so SIGINT/SIGTERM cancel in-flight requests.
func ExecuteErr() error {
	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()
	return rootCmd.ExecuteContext(ctx)
}

func init() {
	rootCmd.PersistentFlags().StringArrayVarP(&flags.agents, "agent", "a", nil,
		"Agent alias or URL (env: AGC_AGENT_URL); repeat to call multiple in parallel")
	rootCmd.PersistentFlags().BoolVar(&flags.allAgents, "all", false,
		"Send to ALL registered agents in parallel")
	rootCmd.PersistentFlags().StringVar(&flags.transport, "transport", "",
		"Preferred transport binding: JSONRPC or HTTP+JSON (A2A spec values; default: auto from agent card)")
	rootCmd.PersistentFlags().StringVar(&flags.fields, "fields", "",
		"Comma-separated field paths to include in output (e.g. \"id,status.state,parts\")")
	rootCmd.PersistentFlags().StringVarP(&flags.format, "format", "f", "",
		"Output format: json, table, yaml (default: table in terminal, json when piped)")
	rootCmd.PersistentFlags().DurationVar(&flags.timeout, "timeout", 0,
		"Request timeout, e.g. 30s, 2m (default: 30s)")
	rootCmd.PersistentFlags().StringVar(&flags.bearerToken, "bearer-token", "",
		"Static bearer token (env: AGC_BEARER_TOKEN) — bypasses OAuth")

	rootCmd.AddCommand(sendCmd)
	rootCmd.AddCommand(taskCmd)
	rootCmd.AddCommand(cardCmd)
}

// --------------------------------------------------------------------------
// AgentTarget
// --------------------------------------------------------------------------

// AgentTarget is a resolved agent endpoint for one invocation.
type AgentTarget struct {
	Alias     string
	URL       string
	OAuth     config.OAuthConfig
	Transport string // "jsonrpc", "rest", or "" for auto-negotiate
}

// Label returns alias if set, otherwise URL.
func (t AgentTarget) Label() string {
	if t.Alias != "" {
		return t.Alias
	}
	return t.URL
}

// --------------------------------------------------------------------------
// Target resolution
// --------------------------------------------------------------------------

func resolveTargets() ([]AgentTarget, error) {
	cfg, _ := config.Load()

	if flags.allAgents {
		if cfg == nil || len(cfg.Agents) == 0 {
			return nil, fmt.Errorf("--all requires at least one registered agent\n  agc agent add <alias> <url>")
		}
		var targets []AgentTarget
		for alias, a := range cfg.Agents {
			targets = append(targets, AgentTarget{Alias: alias, URL: a.URL, OAuth: cfg.ActiveOAuth(alias), Transport: effectiveTransport(a.Transport)})
		}
		return targets, nil
	}

	refs := expandAgentRefs(flags.agents)
	if len(refs) == 0 {
		if env := os.Getenv("AGC_AGENT_URL"); env != "" {
			refs = expandAgentRefs([]string{env})
		}
	}
	if len(refs) > 0 {
		var targets []AgentTarget
		for _, ref := range refs {
			t, err := resolveRef(ref, cfg)
			if err != nil {
				return nil, err
			}
			targets = append(targets, t)
		}
		return targets, nil
	}

	if cfg != nil {
		if agent, ok := cfg.ActiveAgent(); ok {
			alias := cfg.CurrentAgent
			return []AgentTarget{{Alias: alias, URL: agent.URL, OAuth: cfg.ActiveOAuth(alias), Transport: effectiveTransport(cfg.Agents[alias].Transport)}}, nil
		}
	}
	return nil, fmt.Errorf(
		"no agent configured\n" +
			"  Register: agc agent add <alias> <url>\n" +
			"  Or pass:  --agent <alias|url>  /  AGC_AGENT_URL=<url>",
	)
}

func resolveRef(ref string, cfg *config.Config) (AgentTarget, error) {
	if cfg != nil {
		if agent, ok := cfg.ResolveAgent(ref); ok {
			alias := ref
			if config.IsURL(ref) {
				alias = ""
			}
			return AgentTarget{Alias: alias, URL: agent.URL, OAuth: cfg.ActiveOAuth(alias), Transport: effectiveTransport(cfg.Agents[alias].Transport)}, nil
		}
	}
	if err := validate.AgentURL(ref); err != nil {
		return AgentTarget{}, fmt.Errorf("unknown agent %q: %w", ref, err)
	}
	return AgentTarget{URL: ref, Transport: flags.transport}, nil
}

func expandAgentRefs(refs []string) []string {
	var out []string
	for _, r := range refs {
		r = strings.TrimSpace(r)
		if r == "" {
			continue
		}
		if !config.IsURL(r) && strings.Contains(r, ",") {
			for _, p := range strings.Split(r, ",") {
				if p = strings.TrimSpace(p); p != "" {
					out = append(out, p)
				}
			}
		} else {
			out = append(out, r)
		}
	}
	return out
}


// effectiveTransport returns the transport to use: --transport flag > agent config > empty (auto).
func effectiveTransport(agentTransport string) string {
	if flags.transport != "" {
		return normalizeTransport(flags.transport)
	}
	return agentTransport  // already normalized (stored via normalizeTransport in agent add/update)
}
// resolveAgentURL — single URL for commands that can't be multi-agent.
func resolveAgentURL() (string, error) {
	targets, err := resolveTargets()
	if err != nil {
		return "", err
	}
	if len(targets) > 1 {
		return "", fmt.Errorf("this command supports only one agent; use --agent <alias>")
	}
	return targets[0].URL, nil
}

// resolvedAgent — for auth commands.
func resolvedAgent() (agentURL, agentName string, oauth config.OAuthConfig, err error) {
	targets, err := resolveTargets()
	if err != nil {
		return "", "", config.OAuthConfig{}, err
	}
	if len(targets) > 1 {
		return "", "", config.OAuthConfig{}, fmt.Errorf("this command supports only one agent")
	}
	t := targets[0]
	return t.URL, t.Alias, t.OAuth, nil
}

// --------------------------------------------------------------------------
// Printer factory
// --------------------------------------------------------------------------

func printer() *output.Printer {
	return output.New(resolvedFormat(), flags.fields)
}

func resolvedTimeout() time.Duration {
	if flags.timeout != 0 {
		return flags.timeout
	}
	if cfg, err := config.Load(); err == nil {
		if t := cfg.CurrentProfileData().Timeout; t != 0 {
			return t
		}
	}
	return 30 * time.Second
}

func staticBearerToken() string {
	if flags.bearerToken != "" {
		return flags.bearerToken
	}
	return os.Getenv("AGC_BEARER_TOKEN")
}

// --------------------------------------------------------------------------
// Client factory
// --------------------------------------------------------------------------

// resolveTransportProtocol maps a transport name to the A2A protocol constant.
// Returns empty string for auto-negotiate (SDK picks from agent card).
// resolveTransportProtocol maps a user-supplied transport name to the canonical
// A2A spec protocolBinding value (§4.4.6). Returns empty string for auto-negotiate.
// Supported: JSONRPC, HTTP+JSON (gRPC is not supported in this CLI).
func resolveTransportProtocol(name string) a2a.TransportProtocol {
	// Exact A2A spec protocolBinding values only (§4.4.6).
	switch name {
	case "JSONRPC":
		return a2a.TransportProtocolJSONRPC
	case "HTTP+JSON":
		return a2a.TransportProtocolHTTPJSON
	default:
		return ""
	}
}

// normalizeTransport normalizes a transport name to the canonical A2A spec value.
// Accepted: JSONRPC, HTTP+JSON (exact A2A spec protocolBinding values).
// Returns empty string for auto-negotiate.
func normalizeTransport(name string) string {
	switch strings.ToUpper(strings.TrimSpace(name)) {
	case "JSONRPC":
		return "JSONRPC"
	case "HTTP+JSON":
		return "HTTP+JSON"
	default:
		return "" // unknown → auto-negotiate
	}
}

// cmdTimeout returns a child context of cmd.Context() bounded by the configured
// timeout. All RunE functions should use this so SIGINT/SIGTERM is propagated
// to in-flight A2A requests and HTTP calls.
func cmdTimeout(cmd *cobra.Command) (context.Context, context.CancelFunc) {
	return context.WithTimeout(cmd.Context(), resolvedTimeout())
}

// compatCardResolver is a shared resolver that understands both A2A v1.0 and v0.3
// agent card formats. The custom parseAgentCard wraps a2av0 to also preserve
// OAuth2 flows from v0.3 flat-format security schemes ({"type":"oauth2","flows":{...}}).
var compatCardResolver = &agentcard.Resolver{
	CardParser: parseAgentCard,
}

// parseAgentCard handles both A2A v1.0 and v0.3 cards.
// The a2av0 compat layer silently drops OAuth2 flows when converting v0.3 flat
// security schemes ({"type":"oauth2","flows":{...}}) to v1.0 — OAuth2 is set to
// Description-only. We re-extract the flows from the raw JSON and patch them back.
func parseAgentCard(b []byte) (*a2a.AgentCard, error) {
	card, err := a2av0.NewAgentCardParser()(b)
	if err != nil {
		return nil, err
	}
	if len(card.SecuritySchemes) == 0 {
		return card, nil
	}

	// Re-parse only securitySchemes to recover flows that the compat layer lost.
	var raw struct {
		SecuritySchemes map[string]json.RawMessage `json:"securitySchemes"`
	}
	if jerr := json.Unmarshal(b, &raw); jerr != nil {
		return card, nil
	}

	for name, schemeRaw := range raw.SecuritySchemes {
		existing, ok := card.SecuritySchemes[a2a.SecuritySchemeName(name)]
		if !ok {
			continue
		}
		oauth, ok := existing.(a2a.OAuth2SecurityScheme)
		if !ok || oauth.Flows != nil {
			continue // not OAuth2 or flows already present — nothing to patch
		}
		// Parse v0.3 flat format: {"type":"oauth2","flows":{"authorizationCode":{...}}}
		var flat struct {
			Flows map[string]struct {
				AuthorizationURL string            `json:"authorizationUrl"`
				TokenURL         string            `json:"tokenUrl"`
				Scopes           map[string]string `json:"scopes"`
			} `json:"flows"`
		}
		if jerr := json.Unmarshal(schemeRaw, &flat); jerr != nil || len(flat.Flows) == 0 {
			continue
		}
		for flowType, flow := range flat.Flows {
			switch flowType {
			case "authorizationCode":
				oauth.Flows = a2a.AuthorizationCodeOAuthFlow{
					AuthorizationURL: flow.AuthorizationURL,
					TokenURL:         flow.TokenURL,
					Scopes:           flow.Scopes,
				}
			case "clientCredentials":
				oauth.Flows = a2a.ClientCredentialsOAuthFlow{
					TokenURL: flow.TokenURL,
					Scopes:   flow.Scopes,
				}
			case "deviceCode":
				oauth.Flows = a2a.DeviceCodeOAuthFlow{
					TokenURL:               flow.TokenURL,
					DeviceAuthorizationURL: flow.AuthorizationURL,
					Scopes:                 flow.Scopes,
				}
			}
		}
		card.SecuritySchemes[a2a.SecuritySchemeName(name)] = oauth
	}
	return card, nil
}

// newClientForTarget creates an authenticated A2A client following the full
// A2A two-layer card protocol, supporting both v1.0 and v0.3 servers:
//
//  1. Fetch the public card — parsed by the compat parser which handles both
//     v1.0 (`supportedInterfaces`) and v0.3 (`url` + `preferredTransport`).
//     If this fails with a 401/403, the agent's public card itself is protected.
//
//  2. Authenticate using the scheme declared in the public card.
//
//  3. If capabilities.extendedAgentCard is true, fetch the richer card from
//     /extendedAgentCard and update the client (§13.3 A2A spec).
//
// Protocol version selection: the factory registers both v1.0 transports (JSONRPC,
// HTTP+JSON) and the v0.3 JSONRPC compat transport. The SDK selects automatically
// from the `protocolVersion` field in the agent card's `supportedInterfaces`.
func newClientForTarget(ctx context.Context, t AgentTarget) (*a2aclient.Client, error) {
	// Step 1: fetch the public card using the compat parser (handles v0.3 + v1.0).
	card, err := compatCardResolver.Resolve(ctx, t.URL)
	if err != nil {
		// Detect a protected public card (HTTP 401/403). The A2A spec implies
		// the public card should be freely accessible for bootstrap, but some
		// enterprise deployments protect it. Guide the user to bootstrap manually.
		errMsg := err.Error()
		if isAuthError(errMsg) {
			return nil, fmt.Errorf(
				"agent card at %s requires authentication (HTTP 401/403)\n"+
					"The A2A spec implies the public card should be accessible without auth,\n"+
					"but this agent protects it. Bootstrap options:\n"+
					"  --bearer-token <token>    provide a token to fetch the card\n"+
					"  agc agent add %s <url> --client-id <id>   then: agc auth login",
				t.URL, t.Label())
		}
		return nil, fmt.Errorf("fetch agent card from %s: %w", t.URL, err)
	}

	// Step 2: configure transport.
	// The default factory registers v1.0 JSONRPC and HTTP+JSON.
	// We also register the v0.3 JSONRPC compat transport so the factory can
	// connect to agents running the older protocol version — the SDK selects
	// the right one from the card's `protocolVersion` automatically.
	var opts []a2aclient.FactoryOption

	// Always inject client identification headers so servers can track,
	// rate-limit, and audit by client type and version.
	opts = append(opts, a2aclient.WithCallInterceptors(&clientHeadersInterceptor{
		agentAlias: t.Label(),
	}))

	opts = append(opts,
		a2aclient.WithCompatTransport(
			a2av0.Version,
			a2a.TransportProtocolJSONRPC,
			a2av0.NewJSONRPCTransportFactory(a2av0.JSONRPCTransportConfig{}),
		),
	)
	if proto := resolveTransportProtocol(t.Transport); proto != "" {
		opts = append(opts, a2aclient.WithConfig(a2aclient.Config{
			PreferredTransports: []a2a.TransportProtocol{proto},
		}))
	}

	// Step 3: authenticate.
	if token := staticBearerToken(); token != "" {
		opts = append(opts, a2aclient.WithCallInterceptors(&bearerTokenInterceptor{token: token}))
	} else {
		mgr := auth.NewManager(t.URL, card, t.OAuth)
		if mgr.NeedsAuth() {
			tok, authErr := mgr.EnsureToken(ctx, isTerminal())
			if authErr != nil {
				return nil, fmt.Errorf("authentication required — run: agc auth login --agent %s", t.Label())
			}
			opts = append(opts, a2aclient.WithCallInterceptors(mgr.TokenInterceptor(tok)))
		}
	}

	client, err := a2aclient.NewFromCard(ctx, card, opts...)
	if err != nil {
		return nil, fmt.Errorf("create client for %s: %w", t.Label(), err)
	}

	// Step 3: if the agent declares an extended card, fetch it and upgrade.
	// The extended card may have additional skills or org-specific capabilities
	// not visible in the public card (A2A spec §3.1.11, §13.3).
	if card.Capabilities.ExtendedAgentCard {
		extCard, extErr := client.GetExtendedAgentCard(ctx, &a2a.GetExtendedAgentCardRequest{})
		if extErr == nil {
			// Replace the public card with the richer extended card for this session.
			if updateErr := client.UpdateCard(extCard); updateErr != nil {
				// Non-fatal: log and continue with the public card.
				fmt.Fprintf(os.Stderr, "warning: could not apply extended card for %s: %v\n",
					t.Label(), updateErr)
			}
		}
		// If the extended card fetch fails (e.g., token doesn't have permission),
		// silently continue with the public card — don't break the session.
	}

	return client, nil
}

// isAuthError returns true if the error message indicates an HTTP 401/403.
func isAuthError(msg string) bool {
	for _, marker := range []string{"401", "403", "unauthorized", "forbidden", "Unauthorized", "Forbidden"} {
		if strings.Contains(msg, marker) {
			return true
		}
	}
	return false
}

func isTerminal() bool {
	fi, err := os.Stdin.Stat()
	return err == nil && (fi.Mode()&os.ModeCharDevice) != 0
}

type bearerTokenInterceptor struct {
	a2aclient.PassthroughInterceptor
	token string
}

func (b *bearerTokenInterceptor) Before(ctx context.Context, req *a2aclient.Request) (context.Context, any, error) {
	req.ServiceParams.Append("Authorization", "Bearer "+b.token)
	return ctx, nil, nil
}

// clientHeadersInterceptor injects client identification headers into every A2A request.
// Servers can use these to track, rate-limit, and audit by client type and version.
type clientHeadersInterceptor struct {
	a2aclient.PassthroughInterceptor
	agentAlias string
}

func (h *clientHeadersInterceptor) Before(ctx context.Context, req *a2aclient.Request) (context.Context, any, error) {
	req.ServiceParams.Append("User-Agent", "agc/"+version+" (genai.stargate.toyota/a2a/agc)")
	req.ServiceParams.Append("X-Agc-Version", version)
	if h.agentAlias != "" {
		req.ServiceParams.Append("X-Agc-Agent", h.agentAlias)
	}
	return ctx, nil, nil
}

// resolvedFormat returns the effective output format:
// explicit --format flag > auto-detect (json when not TTY, table when TTY).
// This mirrors gws behaviour: json is the default for scripts, table for humans.
func resolvedFormat() string {
	if flags.format != "" {
		return flags.format
	}
	if isTerminal() {
		return "table"
	}
	return "json"
}
