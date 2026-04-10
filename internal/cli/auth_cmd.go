package cli

import (
	"context"
	"fmt"
	"strings"
	"time"

	"github.com/spf13/cobra"
	"genai.stargate.toyota/agc/internal/auth"
	"genai.stargate.toyota/agc/internal/config"
)

var authCmd = &cobra.Command{
	Use:   "auth",
	Short: "Manage authentication — per-agent token storage",
	Long: `Authenticate with registered A2A agents.

Authentication is per-agent: each registered alias has its own token
stored in the system keychain under its URL. Tokens are refreshed
automatically when they expire.`,
}

func init() {
	authCmd.AddCommand(authLoginCmd)
	authCmd.AddCommand(authLogoutCmd)
	authCmd.AddCommand(authStatusCmd)
	rootCmd.AddCommand(authCmd)
}

// ---- auth login ----

var authLoginCmd = &cobra.Command{
	Use:   "login",
	Short: "Authenticate with an agent (auto-detects OAuth flow from agent card)",
	Long: `Authenticate with the active agent (or --agent <alias|url>).

The OAuth flow is auto-detected from the agent card's security schemes:
  - Device Code         — prints URL + code; you authenticate in a browser tab
  - Authorization Code  — opens your browser with a local redirect server
  - Client Credentials  — prompts for client secret
  - HTTP Bearer         — prompts for a static bearer token
  - API Key             — prompts for an API key

Tokens are stored per-agent in the system keychain (macOS Keychain, Linux
Secret Service, Windows Credential Manager) with an AES-256-GCM encrypted
file fallback. Set AGC_KEYRING_BACKEND=file for headless/Docker use.`,
	Example: `  agc auth login                   # active agent
  agc auth login --agent prod
  agc auth login --agent local`,
	RunE: runAuthLogin,
}

func runAuthLogin(cmd *cobra.Command, _ []string) error {
	// Use resolvedAgent() so we get the agent-specific OAuth config.
	agentURL, agentName, oauthCfg, err := resolvedAgent()
	if err != nil {
		return err
	}

	// Short timeout for the card fetch; the browser OAuth flow gets a generous
	// 5 minutes so the user has time to complete the login in the browser.
	cardCtx, cardCancel := cmdTimeout(cmd)
	defer cardCancel()

	card, err := compatCardResolver.Resolve(cardCtx, agentURL)
	if err != nil {
		return fmt.Errorf("fetch agent card from %s: %w", agentURL, err)
	}

	mgr := auth.NewManager(agentURL, card, oauthCfg)

	if !mgr.NeedsAuth() {
		label := agentURL
		if agentName != "" {
			label = fmt.Sprintf("%s (%s)", agentName, agentURL)
		}
		fmt.Printf("Agent %s does not require authentication.\n", label)
		return nil
	}

	loginCtx, loginCancel := context.WithTimeout(cmd.Context(), 5*time.Minute)
	defer loginCancel()

	if _, err := mgr.Login(loginCtx); err != nil {
		return fmt.Errorf("login failed: %w", err)
	}

	if agentName != "" {
		fmt.Printf("Authenticated as %q (%s)\n", agentName, agentURL)
	}
	return nil
}

// ---- auth logout ----

var authLogoutCmd = &cobra.Command{
	Use:   "logout",
	Short: "Remove stored credentials for an agent",
	Example: `  agc auth logout                  # active agent
  agc auth logout --agent prod`,
	RunE: func(_ *cobra.Command, _ []string) error {
		agentURL, agentName, _, err := resolvedAgent()
		if err != nil {
			return err
		}
		if err := auth.DeleteToken(agentURL); err != nil {
			return fmt.Errorf("logout failed: %w", err)
		}
		label := agentURL
		if agentName != "" {
			label = fmt.Sprintf("%q (%s)", agentName, agentURL)
		}
		fmt.Printf("Credentials removed for %s\n", label)
		return nil
	},
}

// ---- auth status ----

var authStatusCmd = &cobra.Command{
	Use:   "status",
	Short: "Show authentication status — all agents or a specific one",
	Long: `Show authentication status for registered agents.

Without --agent, shows status for ALL registered agents.
With --agent <alias>, shows status for that specific agent only.`,
	Example: `  agc auth status               # all agents
  agc auth status --agent prod  # specific agent`,
	RunE: runAuthStatus,
}

func runAuthStatus(_ *cobra.Command, _ []string) error {
	p := printer()

	// If --agent is specified, show only that agent.
	if len(flags.agents) > 0 {
		agentURL, agentName, _, err := resolvedAgent()
		if err != nil {
			return err
		}
		status, err := agentAuthStatus(agentURL, agentName)
		if err != nil {
			return err
		}
		return p.Print(status)
	}

	// No --agent: show all registered agents.
	cfg, err := config.Load()
	if err != nil {
		return fmt.Errorf("load config: %w", err)
	}

	if len(cfg.Agents) == 0 {
		p.PrintLine("No agents registered. Use: agc agent add <alias> <url>")
		return nil
	}

	var statuses []map[string]any
	for alias, a := range cfg.Agents {
		status, err := agentAuthStatus(a.URL, alias)
		if err != nil {
			status = map[string]any{
				"alias": alias,
				"url":   a.URL,
				"error": err.Error(),
			}
		}
		statuses = append(statuses, status)
	}
	return p.Print(statuses)
}

// agentAuthStatus returns the auth status map for a single agent.
func agentAuthStatus(agentURL, alias string) (map[string]any, error) {
	token, err := auth.LoadToken(agentURL)
	if err != nil {
		return nil, fmt.Errorf("read token for %s: %w", agentURL, err)
	}

	info := map[string]any{
		"alias":         alias,
		"url":           agentURL,
		"authenticated": false,
	}
	if token == nil {
		return info, nil
	}

	info["authenticated"] = token.Valid()
	info["token_type"] = token.TokenType
	info["has_refresh"] = token.CanRefresh()
	if len(token.Scopes) > 0 {
		info["scopes"] = token.Scopes
	}
	if !token.ExpiresAt.IsZero() {
		info["expires_at"] = token.ExpiresAt.Format(time.RFC3339)
		if token.Valid() {
			info["expires_in"] = fmt.Sprintf("%.0fm", time.Until(token.ExpiresAt).Minutes())
		} else {
			info["expired"] = true
		}
	}
	// Mask the token — first/last 4 chars only.
	if at := token.AccessToken; len(at) > 8 {
		info["access_token"] = at[:4] + strings.Repeat("*", len(at)-8) + at[len(at)-4:]
	} else if at != "" {
		info["access_token"] = "****"
	}
	return info, nil
}

// ---- auth set-secret ----

var authSetSecretCmd = &cobra.Command{
	Use:   "set-secret",
	Short: "Store the OAuth client secret for an agent in the keychain",
	Long: `Store an OAuth client secret securely in the system keychain.

The secret is used for the Client Credentials OAuth flow (machine-to-machine).
It is stored in the keychain — never in the config file.

For CI/non-interactive environments, set AGC_CLIENT_SECRET env var instead.`,
	Example: `  agc auth set-secret                  # active agent, prompts for secret
  agc auth set-secret --agent prod      # specific agent`,
	RunE: func(_ *cobra.Command, _ []string) error {
		agentURL, _, _, err := resolvedAgent()
		if err != nil {
			return err
		}
		secret := auth.PromptSecret(fmt.Sprintf("Client secret for %s: ", agentURL))
		if secret == "" {
			return fmt.Errorf("secret cannot be empty")
		}
		if err := auth.SaveClientSecret(agentURL, secret); err != nil {
			return fmt.Errorf("save client secret: %w", err)
		}
		fmt.Println("Client secret stored in keychain.")
		return nil
	},
}

// ---- auth delete-secret ----

var authDeleteSecretCmd = &cobra.Command{
	Use:   "delete-secret",
	Short: "Remove the stored OAuth client secret for an agent",
	Example: `  agc auth delete-secret
  agc auth delete-secret --agent prod`,
	RunE: func(_ *cobra.Command, _ []string) error {
		agentURL, _, _, err := resolvedAgent()
		if err != nil {
			return err
		}
		if err := auth.DeleteClientSecret(agentURL); err != nil {
			return fmt.Errorf("delete client secret: %w", err)
		}
		fmt.Printf("Client secret removed for %s\n", agentURL)
		return nil
	},
}

func init() {
	authCmd.AddCommand(authSetSecretCmd)
	authCmd.AddCommand(authDeleteSecretCmd)
}
