package auth

// OAuth flows for agc, modelled after the MCP Go SDK auth package.
// Uses golang.org/x/oauth2 for token exchange, refresh, and PKCE — no manual HTTP.
//
// Registration priority (per MCP SDK pattern):
//   1. CIMD  — client_id IS a URL; server fetches the metadata doc
//   2. Pre-registered — client_id + client_secret already known
//   3. DCR   — register dynamically with the server's /register endpoint

import (
	"context"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"os/exec"
	"runtime"
	"strings"
	"time"

	"github.com/a2aproject/a2a-go/v2/a2a"
	"golang.org/x/oauth2"
)

// --------------------------------------------------------------------------
// Device Code flow — best for CLI (no local callback server needed)
// --------------------------------------------------------------------------

type deviceAuthResponse struct {
	DeviceCode              string `json:"device_code"`
	UserCode                string `json:"user_code"`
	VerificationURI         string `json:"verification_uri"`
	VerificationURIComplete string `json:"verification_uri_complete"`
	ExpiresIn               int    `json:"expires_in"`
	Interval                int    `json:"interval"`
	Error                   string `json:"error"`
	ErrorDesc               string `json:"error_description"`
}

// RunDeviceCodeFlow executes the OAuth 2.0 Device Authorization Grant (RFC 8628).
func RunDeviceCodeFlow(ctx context.Context, flow a2a.DeviceCodeOAuthFlow, clientID string, scopes []string) (*TokenSet, error) {
	endpoint := oauth2.Endpoint{
		DeviceAuthURL: flow.DeviceAuthorizationURL,
		TokenURL:      flow.TokenURL,
		AuthStyle:     oauth2.AuthStyleInParams,
	}
	cfg := &oauth2.Config{
		ClientID: clientID,
		Endpoint: endpoint,
		Scopes:   scopes,
	}

	// Request device authorization.
	resp, err := cfg.DeviceAuth(ctx)
	if err != nil {
		return nil, fmt.Errorf("device auth: %w", err)
	}

	// Show the user what to do.
	uri := resp.VerificationURIComplete
	if uri == "" {
		uri = resp.VerificationURI
	}
	fmt.Printf("\nTo authenticate, visit:\n\n  %s\n\nAnd enter code: %s\n\n", resp.VerificationURI, resp.UserCode)
	if resp.VerificationURIComplete != "" {
		fmt.Printf("Or open the complete link:\n  %s\n\n", resp.VerificationURIComplete)
	}
	openBrowserIfAvailable(uri)

	// Poll until the user completes authorization.
	token, err := cfg.DeviceAccessToken(ctx, resp)
	if err != nil {
		return nil, fmt.Errorf("device token poll: %w", err)
	}
	return tokenToTokenSet(token, scopes), nil
}

// --------------------------------------------------------------------------
// Authorization Code + PKCE — browser-based
// --------------------------------------------------------------------------

// RunAuthCodeFlow executes the OAuth 2.0 Authorization Code flow with PKCE.
// Starts a local HTTP server on a free port to receive the redirect callback.
func RunAuthCodeFlow(ctx context.Context, flow a2a.AuthorizationCodeOAuthFlow, clientID string, scopes []string) (*TokenSet, error) {
	// Pick a free local port for the redirect callback.
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return nil, fmt.Errorf("start callback server: %w", err)
	}
	port := listener.Addr().(*net.TCPAddr).Port
	redirectURI := fmt.Sprintf("http://localhost:%d/callback", port)

	cfg := &oauth2.Config{
		ClientID:    clientID,
		RedirectURL: redirectURI,
		Endpoint: oauth2.Endpoint{
			AuthURL:   flow.AuthorizationURL,
			TokenURL:  flow.TokenURL,
			AuthStyle: oauth2.AuthStyleInParams,
		},
		Scopes: scopes,
	}

	// Generate PKCE verifier and state using golang.org/x/oauth2 helpers.
	verifier := oauth2.GenerateVerifier()
	state := generateState()

	// Build the authorization URL.
	authURL := cfg.AuthCodeURL(state, oauth2.S256ChallengeOption(verifier))

	// Start the callback server.
	codeCh := make(chan string, 1)
	errCh := make(chan error, 1)
	mux := http.NewServeMux()
	srv := &http.Server{Handler: mux}

	mux.HandleFunc("/callback", func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Query().Get("state") != state {
			http.Error(w, "state mismatch", http.StatusBadRequest)
			errCh <- fmt.Errorf("oauth state mismatch — possible CSRF")
			return
		}
		if e := r.URL.Query().Get("error"); e != "" {
			msg := e + ": " + r.URL.Query().Get("error_description")
			http.Error(w, msg, http.StatusBadRequest)
			errCh <- fmt.Errorf("authorization denied: %s", msg)
			return
		}
		code := r.URL.Query().Get("code")
		if code == "" {
			http.Error(w, "missing code", http.StatusBadRequest)
			errCh <- fmt.Errorf("no authorization code in callback")
			return
		}
		fmt.Fprintln(w, "<html><body><p>Authentication successful. You can close this window.</p></body></html>")
		codeCh <- code
	})

	go func() {
		if err := srv.Serve(listener); err != nil && err != http.ErrServerClosed {
			select {
			case errCh <- err:
			default:
			}
		}
	}()

	fmt.Printf("\nOpening browser for authentication...\n  %s\n\n", authURL)
	openBrowserIfAvailable(authURL)
	fmt.Println("Waiting for authorization (complete in your browser)...")

	var code string
	select {
	case <-ctx.Done():
		srv.Close()
		return nil, ctx.Err()
	case e := <-errCh:
		srv.Close()
		return nil, e
	case code = <-codeCh:
		srv.Close()
	}

	// Exchange the code for a token using golang.org/x/oauth2.
	clientCtx := context.WithValue(ctx, oauth2.HTTPClient, http.DefaultClient)
	token, err := cfg.Exchange(clientCtx, code, oauth2.VerifierOption(verifier))
	if err != nil {
		return nil, fmt.Errorf("token exchange: %w", err)
	}
	return tokenToTokenSet(token, scopes), nil
}

// --------------------------------------------------------------------------
// Client Credentials — machine-to-machine
// --------------------------------------------------------------------------

// RunClientCredentialsFlow executes the OAuth 2.0 Client Credentials grant.
func RunClientCredentialsFlow(ctx context.Context, flow a2a.ClientCredentialsOAuthFlow, clientID, clientSecret string, scopes []string) (*TokenSet, error) {
	cfg := &oauth2.Config{
		ClientID:     clientID,
		ClientSecret: clientSecret,
		Endpoint: oauth2.Endpoint{
			TokenURL:  flow.TokenURL,
			AuthStyle: oauth2.AuthStyleInParams,
		},
		Scopes: scopes,
	}

	// Client credentials: use a background context since no user interaction.
	token, err := cfg.Exchange(ctx, "",
		oauth2.SetAuthURLParam("grant_type", "client_credentials"),
	)
	if err != nil {
		return nil, fmt.Errorf("client credentials: %w", err)
	}
	return tokenToTokenSet(token, scopes), nil
}

// --------------------------------------------------------------------------
// Token refresh
// --------------------------------------------------------------------------

// RefreshAccessToken exchanges a refresh token for a new access token.
func RefreshAccessToken(ctx context.Context, tokenURL, clientID, refreshToken string) (*TokenSet, error) {
	cfg := &oauth2.Config{
		ClientID: clientID,
		Endpoint: oauth2.Endpoint{
			TokenURL:  tokenURL,
			AuthStyle: oauth2.AuthStyleInParams,
		},
	}

	// Create a token with only the refresh token set and use TokenSource to refresh.
	existing := &oauth2.Token{RefreshToken: refreshToken}
	ts := cfg.TokenSource(ctx, existing)
	token, err := ts.Token()
	if err != nil {
		return nil, fmt.Errorf("token refresh: %w", err)
	}
	return tokenToTokenSet(token, nil), nil
}

// --------------------------------------------------------------------------
// OIDC discovery
// --------------------------------------------------------------------------

// OIDCMetadata holds the relevant subset of an OIDC provider metadata document.
type OIDCMetadata struct {
	AuthorizationEndpoint       string `json:"authorization_endpoint"`
	TokenEndpoint               string `json:"token_endpoint"`
	DeviceAuthorizationEndpoint string `json:"device_authorization_endpoint"`
}

// FetchOIDCMetadata retrieves the OIDC provider metadata from the discovery URL.
func FetchOIDCMetadata(ctx context.Context, discoveryURL string) (*OIDCMetadata, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, discoveryURL, nil)
	if err != nil {
		return nil, err
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, fmt.Errorf("fetch OIDC metadata from %s: %w", discoveryURL, err)
	}
	defer resp.Body.Close()

	var meta OIDCMetadata
	if err := decodeJSON(resp.Body, &meta); err != nil {
		return nil, fmt.Errorf("parse OIDC metadata: %w", err)
	}
	return &meta, nil
}

// --------------------------------------------------------------------------
// Helpers
// --------------------------------------------------------------------------

// tokenToTokenSet converts an oauth2.Token to our internal TokenSet.
func tokenToTokenSet(t *oauth2.Token, requestedScopes []string) *TokenSet {
	ts := &TokenSet{
		AccessToken:  t.AccessToken,
		RefreshToken: t.RefreshToken,
		TokenType:    t.TokenType,
		Scopes:       requestedScopes,
	}
	if !t.Expiry.IsZero() {
		ts.ExpiresAt = t.Expiry
	}
	// Parse scope from extra if available.
	if scope, ok := t.Extra("scope").(string); ok && scope != "" {
		ts.Scopes = strings.Fields(scope)
	}
	return ts
}

func generateState() string {
	b := make([]byte, 16)
	if _, err := rand.Read(b); err != nil {
		return fmt.Sprintf("%d", time.Now().UnixNano())
	}
	return base64.RawURLEncoding.EncodeToString(b)
}

func openBrowserIfAvailable(rawURL string) {
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

func decodeJSON(r io.Reader, v any) error {
	return json.NewDecoder(r).Decode(v)
}
