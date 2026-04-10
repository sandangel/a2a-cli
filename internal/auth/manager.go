package auth

import (
	"context"
	"fmt"
	"os"
	"strings"

	"github.com/a2aproject/a2a-go/v2/a2a"
	"github.com/a2aproject/a2a-go/v2/a2aclient"
	"genai.stargate.toyota/agc/internal/config"
)

// Manager handles authentication for a single agent.
// It inspects the AgentCard's security requirements, loads/refreshes tokens
// from the keychain, and triggers interactive login flows when needed.
type Manager struct {
	agentURL string
	card     *a2a.AgentCard
	oauth    config.OAuthConfig
}

// NewManager creates an auth Manager for the given agent.
func NewManager(agentURL string, card *a2a.AgentCard, oauth config.OAuthConfig) *Manager {
	return &Manager{agentURL: agentURL, card: card, oauth: oauth}
}

// NeedsAuth returns true if the agent card declares security requirements.
func (m *Manager) NeedsAuth() bool {
	return len(m.card.SecurityRequirements) > 0 || len(m.card.SecuritySchemes) > 0
}

// EnsureToken returns a valid TokenSet, refreshing or triggering a login flow
// as needed. Set interactive=true when running in a terminal; false returns an
// error if no stored token exists.
func (m *Manager) EnsureToken(ctx context.Context, interactive bool) (*TokenSet, error) {
	token, err := LoadToken(m.agentURL)
	if err != nil {
		return nil, fmt.Errorf("load stored token: %w", err)
	}

	// Token is present and valid — use it.
	if token.Valid() {
		return token, nil
	}

	// Expired but refreshable — try refresh first.
	if token.CanRefresh() {
		tokenURL := m.tokenURLForRefresh()
		if tokenURL != "" {
			refreshed, refreshErr := RefreshAccessToken(ctx, tokenURL, m.oauth.ClientID, token.RefreshToken)
			if refreshErr == nil {
				// Preserve refresh token if not rotated.
				if refreshed.RefreshToken == "" {
					refreshed.RefreshToken = token.RefreshToken
				}
				if saveErr := SaveToken(m.agentURL, refreshed); saveErr != nil {
					fmt.Fprintf(os.Stderr, "warning: could not save refreshed token: %v\n", saveErr)
				}
				return refreshed, nil
			}
			// Refresh failed — fall through to re-login.
			fmt.Printf("Token refresh failed (%v), re-authenticating...\n", refreshErr)
		}
	}

	if !interactive {
		return nil, fmt.Errorf("authentication required — run: a2a auth login")
	}

	return m.Login(ctx)
}

// Login detects the best OAuth flow from the agent card and executes it.
func (m *Manager) Login(ctx context.Context) (*TokenSet, error) {
	schemeName, scheme, err := m.selectScheme()
	if err != nil {
		return nil, err
	}

	fmt.Printf("Authenticating with scheme %q...\n", schemeName)

	token, err := m.runFlow(ctx, scheme)
	if err != nil {
		return nil, err
	}

	if err := SaveToken(m.agentURL, token); err != nil {
		fmt.Fprintf(os.Stderr, "warning: could not save token: %v\n", err)
	}

	fmt.Println("Authentication successful.")
	return token, nil
}

// TokenInterceptor returns a CallInterceptor that injects the given token into
// every outgoing request according to the agent's security scheme.
func (m *Manager) TokenInterceptor(token *TokenSet) a2aclient.CallInterceptor {
	_, scheme, _ := m.selectScheme()
	return &tokenInjector{token: token, scheme: scheme}
}

// --------------------------------------------------------------------------
// Internal helpers
// --------------------------------------------------------------------------

// selectScheme picks the first suitable security scheme from the agent card
// in preference order: OAuth2 (deviceCode > authorizationCode > clientCredentials)
// > OpenIDConnect > HTTPAuth > APIKey.
func (m *Manager) selectScheme() (a2a.SecuritySchemeName, a2a.SecurityScheme, error) {
	if m.card == nil || len(m.card.SecuritySchemes) == 0 {
		return "", nil, fmt.Errorf("agent card has no security schemes")
	}

	// Collect requirements from card or fall back to all declared schemes.
	candidates := m.schemeCandidates()

	// Priority 1: OAuth2 with deviceCode flow.
	for name, s := range candidates {
		if oauth, ok := s.(a2a.OAuth2SecurityScheme); ok {
			if _, ok := oauth.Flows.(a2a.DeviceCodeOAuthFlow); ok {
				return name, s, nil
			}
		}
	}
	// Priority 2: OAuth2 with authorizationCode flow.
	for name, s := range candidates {
		if oauth, ok := s.(a2a.OAuth2SecurityScheme); ok {
			if _, ok := oauth.Flows.(a2a.AuthorizationCodeOAuthFlow); ok {
				return name, s, nil
			}
		}
	}
	// Priority 3: OAuth2 with clientCredentials.
	for name, s := range candidates {
		if oauth, ok := s.(a2a.OAuth2SecurityScheme); ok {
			if _, ok := oauth.Flows.(a2a.ClientCredentialsOAuthFlow); ok {
				return name, s, nil
			}
		}
	}
	// Priority 4: OpenID Connect.
	for name, s := range candidates {
		if _, ok := s.(a2a.OpenIDConnectSecurityScheme); ok {
			return name, s, nil
		}
	}
	// Priority 5: HTTP Bearer.
	for name, s := range candidates {
		if _, ok := s.(a2a.HTTPAuthSecurityScheme); ok {
			return name, s, nil
		}
	}
	// Priority 6: API Key.
	for name, s := range candidates {
		if _, ok := s.(a2a.APIKeySecurityScheme); ok {
			return name, s, nil
		}
	}
	return "", nil, fmt.Errorf("no supported security scheme found in agent card")
}

// schemeCandidates returns the subset of SecuritySchemes that satisfy at least
// one of the agent card's SecurityRequirements (or all schemes if no explicit
// requirements are declared).
func (m *Manager) schemeCandidates() map[a2a.SecuritySchemeName]a2a.SecurityScheme {
	if len(m.card.SecurityRequirements) == 0 {
		return m.card.SecuritySchemes
	}
	result := make(map[a2a.SecuritySchemeName]a2a.SecurityScheme)
	for _, req := range m.card.SecurityRequirements {
		for name := range req {
			if s, ok := m.card.SecuritySchemes[name]; ok {
				result[name] = s
			}
		}
	}
	if len(result) == 0 {
		return m.card.SecuritySchemes
	}
	return result
}

// runFlow executes the appropriate authentication flow for the given scheme.
func (m *Manager) runFlow(ctx context.Context, scheme a2a.SecurityScheme) (*TokenSet, error) {
	scopes := m.effectiveScopes(scheme)
	clientID := m.oauth.ClientID

	switch s := scheme.(type) {
	case a2a.OAuth2SecurityScheme:
		return m.runOAuth2Flow(ctx, s, clientID, scopes)

	case a2a.OpenIDConnectSecurityScheme:
		return m.runOIDCFlow(ctx, s, clientID, scopes)

	case a2a.HTTPAuthSecurityScheme:
		token := PromptSecret(fmt.Sprintf("Enter Bearer token for %s: ", m.agentURL))
		return &TokenSet{AccessToken: token, TokenType: "bearer"}, nil

	case a2a.APIKeySecurityScheme:
		key := PromptSecret(fmt.Sprintf("Enter API key (%s) for %s: ", s.Name, m.agentURL))
		return &TokenSet{AccessToken: key}, nil

	default:
		return nil, fmt.Errorf("unsupported security scheme type %T", scheme)
	}
}

func (m *Manager) runOAuth2Flow(ctx context.Context, s a2a.OAuth2SecurityScheme, clientID string, scopes []string) (*TokenSet, error) {
	// client_id can be a regular client ID or a CIMD URL.
	// Prefer setting it in config: agc agent update <alias> --client-id <id>
	if clientID == "" {
		fmt.Printf("Tip: save client_id permanently with: agc agent update <alias> --client-id <id>\n")
		fmt.Printf("     e.g. agc agent update <alias> --client-id <your-client-id>\n")
		clientID = promptInput("OAuth Client ID: ")
		if clientID == "" {
			return nil, fmt.Errorf("client_id is required")
		}
	}
	switch flow := s.Flows.(type) {
	case a2a.DeviceCodeOAuthFlow:
		return RunDeviceCodeFlow(ctx, flow, clientID, scopes)
	case a2a.AuthorizationCodeOAuthFlow:
		return RunAuthCodeFlow(ctx, flow, clientID, scopes)
	case a2a.ClientCredentialsOAuthFlow:
		// Look up client secret: AGC_CLIENT_SECRET env > keychain > interactive prompt.
		clientSecret, secretErr := LoadClientSecret(m.agentURL)
		if secretErr != nil {
			return nil, fmt.Errorf("load client secret: %w", secretErr)
		}
		if clientSecret == "" {
			clientSecret = PromptSecret(fmt.Sprintf("OAuth Client Secret for %s: ", m.agentURL))
			if clientSecret == "" {
				return nil, fmt.Errorf("client secret is required for ClientCredentials flow")
			}
			// Offer to save it for future use.
			if isTerminal() {
				if reply := promptInput("Save to keychain for future use? [y/N]: "); strings.EqualFold(reply, "y") || strings.EqualFold(reply, "yes") {
					_ = SaveClientSecret(m.agentURL, clientSecret)
				}
			}
		}
		return RunClientCredentialsFlow(ctx, flow, clientID, clientSecret, scopes)
	default:
		return nil, fmt.Errorf("unsupported OAuth flow type %T — use authorizationCode, deviceCode, or clientCredentials", s.Flows)
	}
}



func (m *Manager) runOIDCFlow(ctx context.Context, s a2a.OpenIDConnectSecurityScheme, clientID string, scopes []string) (*TokenSet, error) {
	meta, err := FetchOIDCMetadata(ctx, s.OpenIDConnectURL)
	if err != nil {
		return nil, err
	}
	if clientID == "" {
		clientID = promptInput("OAuth Client ID: ")
	}
	// Prefer device code if the OIDC provider supports it.
	if meta.DeviceAuthorizationEndpoint != "" {
		flow := a2a.DeviceCodeOAuthFlow{
			DeviceAuthorizationURL: meta.DeviceAuthorizationEndpoint,
			TokenURL:               meta.TokenEndpoint,
		}
		return RunDeviceCodeFlow(ctx, flow, clientID, scopes)
	}
	// Fall back to Authorization Code + PKCE.
	flow := a2a.AuthorizationCodeOAuthFlow{
		AuthorizationURL: meta.AuthorizationEndpoint,
		TokenURL:         meta.TokenEndpoint,
		PKCERequired:     true,
	}
	return RunAuthCodeFlow(ctx, flow, clientID, scopes)
}

// tokenURLForRefresh finds the token endpoint URL from the agent card's schemes.
func (m *Manager) tokenURLForRefresh() string {
	_, scheme, err := m.selectScheme()
	if err != nil {
		return ""
	}
	switch s := scheme.(type) {
	case a2a.OAuth2SecurityScheme:
		switch flow := s.Flows.(type) {
		case a2a.DeviceCodeOAuthFlow:
			if flow.RefreshURL != "" {
				return flow.RefreshURL
			}
			return flow.TokenURL
		case a2a.AuthorizationCodeOAuthFlow:
			if flow.RefreshURL != "" {
				return flow.RefreshURL
			}
			return flow.TokenURL
		case a2a.ClientCredentialsOAuthFlow:
			return flow.TokenURL
		}
	}
	return ""
}

// effectiveScopes returns the scopes to request: config-defined scopes first,
// then falls back to all scopes declared in the scheme.
func (m *Manager) effectiveScopes(scheme a2a.SecurityScheme) []string {
	if len(m.oauth.Scopes) > 0 {
		return m.oauth.Scopes
	}
	switch s := scheme.(type) {
	case a2a.OAuth2SecurityScheme:
		return schemeScopes(s.Flows)
	}
	return nil
}

func schemeScopes(flows a2a.OAuthFlows) []string {
	var scopeMap map[string]string
	switch f := flows.(type) {
	case a2a.DeviceCodeOAuthFlow:
		scopeMap = f.Scopes
	case a2a.AuthorizationCodeOAuthFlow:
		scopeMap = f.Scopes
	case a2a.ClientCredentialsOAuthFlow:
		scopeMap = f.Scopes
	case a2a.ImplicitOAuthFlow:
		scopeMap = f.Scopes
	case a2a.PasswordOAuthFlow:
		scopeMap = f.Scopes
	}
	scopes := make([]string, 0, len(scopeMap))
	for s := range scopeMap {
		scopes = append(scopes, s)
	}
	return scopes
}

// --------------------------------------------------------------------------
// Token injector (CallInterceptor)
// --------------------------------------------------------------------------

type tokenInjector struct {
	a2aclient.PassthroughInterceptor
	token  *TokenSet
	scheme a2a.SecurityScheme
}

func (t *tokenInjector) Before(ctx context.Context, req *a2aclient.Request) (context.Context, any, error) {
	if t.token == nil || t.token.AccessToken == "" {
		return ctx, nil, nil
	}
	switch s := t.scheme.(type) {
	case a2a.APIKeySecurityScheme:
		req.ServiceParams.Append(s.Name, t.token.AccessToken)
	default:
		// HTTP Bearer, OAuth2, OpenIDConnect all use Authorization header.
		tokenType := t.token.TokenType
		if tokenType == "" || strings.EqualFold(tokenType, "bearer") {
			tokenType = "Bearer"
		}
		req.ServiceParams.Append("Authorization", tokenType+" "+t.token.AccessToken)
	}
	return ctx, nil, nil
}

// isTerminal returns true when stdin is a real terminal (not piped).
func isTerminal() bool {
	fi, err := os.Stdin.Stat()
	return err == nil && (fi.Mode()&os.ModeCharDevice) != 0
}

// --------------------------------------------------------------------------
// Interactive prompts
// --------------------------------------------------------------------------

func promptInput(prompt string) string {
	fmt.Print(prompt)
	var input string
	fmt.Scanln(&input)
	return strings.TrimSpace(input)
}

func PromptSecret(prompt string) string {
	fmt.Print(prompt)
	// Use terminal echo suppression on Unix if available.
	// For simplicity and zero extra deps, we read from stdin directly.
	// In production, consider golang.org/x/term.ReadPassword.
	var input string
	fmt.Scanln(&input)
	fmt.Println()
	return strings.TrimSpace(input)
}
