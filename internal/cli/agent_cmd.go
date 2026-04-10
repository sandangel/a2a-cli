package cli

import (
	"fmt"
	"os"

	"github.com/spf13/cobra"
	"genai.stargate.toyota/agc/internal/auth"
	"genai.stargate.toyota/agc/internal/config"
	"genai.stargate.toyota/agc/internal/validate"
)

var agentCmd = &cobra.Command{
	Use:   "agent",
	Short: "Manage named agent aliases",
	Long: `Register, list, switch, and remove named A2A agent aliases.

An alias is a short name (e.g. "prod", "local") that maps to an agent URL
plus optional OAuth settings. Once registered, use the alias anywhere you
would use a URL:

  agc --agent prod send "Hello"
  agc --agent local card`,
}

func init() {
	agentCmd.AddCommand(agentAddCmd)
	agentCmd.AddCommand(agentUseCmd)
	agentCmd.AddCommand(agentListCmd)
	agentCmd.AddCommand(agentRemoveCmd)
	agentCmd.AddCommand(agentShowCmd)
	rootCmd.AddCommand(agentCmd)
}

// ---- agent add ----

var agentAddFlags struct {
	description  string
	clientID     string
	clientSecret string
	scopes       []string
	transport    string
}

var agentAddCmd = &cobra.Command{
	Use:   "add <alias> <url>",
	Short: "Register a named agent alias",
	Long: `Register a short alias that maps to an agent URL.

If the alias already exists it is updated. The OAuth client ID and scopes
are stored in the config file. The client secret (if needed) is stored
separately in the keychain via 'agc auth login'.`,
	Example: `  agc agent add prod https://agent.example.com
  agc agent add local http://localhost:8080 --description "local dev"
  agc agent add staging https://staging.example.com --client-id my-app`,
	Args: cobra.ExactArgs(2),
	RunE: runAgentAdd,
}

func init() {
	agentAddCmd.Flags().StringVar(&agentAddFlags.description, "description", "",
		"Human-readable label for this agent")
	agentAddCmd.Flags().StringVar(&agentAddFlags.clientID, "client-id", "",
		"OAuth client ID for this agent")
	agentAddCmd.Flags().StringSliceVar(&agentAddFlags.scopes, "scopes", nil,
		"OAuth scopes for this agent (comma-separated)")
	agentAddCmd.Flags().StringVar(&agentAddFlags.clientSecret, "client-secret", "",
		"OAuth client secret (stored in keychain, not config file; use AGC_CLIENT_SECRET env var in CI)")
	agentAddCmd.Flags().StringVar(&agentAddFlags.transport, "transport", "",
		"Preferred transport binding: JSONRPC or HTTP+JSON (A2A spec values; default: auto from agent card)")
}

func runAgentAdd(_ *cobra.Command, args []string) error {
	alias, rawURL := args[0], args[1]

	if err := validate.DangerousInput(alias, "alias"); err != nil {
		return err
	}
	if err := validate.AgentURL(rawURL); err != nil {
		return err
	}
	if agentAddFlags.clientID != "" {
		if err := validate.DangerousInput(agentAddFlags.clientID, "--client-id"); err != nil {
			return err
		}
	}

	cfg, err := config.Load()
	if err != nil {
		return err
	}

	agent := config.Agent{
		URL:         rawURL,
		Description: agentAddFlags.description,
		Transport:   normalizeTransport(agentAddFlags.transport),
	}
	if agentAddFlags.clientID != "" || len(agentAddFlags.scopes) > 0 {
		agent.OAuth = config.OAuthConfig{
			ClientID: agentAddFlags.clientID,
			Scopes:   agentAddFlags.scopes,
		}
	}

	isNew := cfg.CurrentAgent == ""
	cfg.SetAgent(alias, agent)

	// Auto-activate if this is the first registered agent.
	if isNew {
		cfg.CurrentAgent = alias
	}
	if err := config.Save(cfg); err != nil {
		return err
	}

	// Store client secret in keychain (never in config file).
	if agentAddFlags.clientSecret != "" {
		if saveErr := auth.SaveClientSecret(rawURL, agentAddFlags.clientSecret); saveErr != nil {
			fmt.Fprintf(os.Stderr, "warning: could not save client secret to keychain: %v\n", saveErr)
		} else {
			fmt.Println("Client secret stored in keychain.")
		}
	}

	fmt.Printf("Agent %q → %s\n", alias, rawURL)
	if isNew {
		fmt.Printf("Set %q as the active agent.\n", alias)
	}
	return nil
}

// ---- agent use ----

var agentUseCmd = &cobra.Command{
	Use:   "use <alias>",
	Short: "Set the active agent alias",
	Example: `  agc agent use prod
  agc agent use local`,
	Args: cobra.ExactArgs(1),
	RunE: func(_ *cobra.Command, args []string) error {
		alias := args[0]
		cfg, err := config.Load()
		if err != nil {
			return err
		}
		if _, ok := cfg.Agents[alias]; !ok {
			return fmt.Errorf("unknown alias %q — register it with: agc agent add %s <url>", alias, alias)
		}
		cfg.CurrentAgent = alias
		if err := config.Save(cfg); err != nil {
			return err
		}
		fmt.Printf("Active agent: %q (%s)\n", alias, cfg.Agents[alias].URL)
		return nil
	},
}

// ---- agent list ----

var agentListCmd = &cobra.Command{
	Use:   "list",
	Short: "List all registered agent aliases",
	Example: `  agc agent list
  agc agent list --format table`,
	RunE: func(_ *cobra.Command, _ []string) error {
		p := printer()
		cfg, err := config.Load()
		if err != nil {
			return err
		}
		if len(cfg.Agents) == 0 {
			p.PrintLine("No agents registered. Use: agc agent add <alias> <url>")
			return nil
		}

		type entry struct {
			Alias       string `json:"alias"`
			URL         string `json:"url"`
			Active      bool   `json:"active"`
			Transport   string `json:"transport,omitempty"`
			Description string `json:"description,omitempty"`
			ClientID    string `json:"client_id,omitempty"`
		}
		var entries []entry
		for name, a := range cfg.Agents {
			entries = append(entries, entry{
				Alias:       name,
				URL:         a.URL,
				Active:      name == cfg.CurrentAgent,
				Transport:   a.Transport,
				Description: a.Description,
				ClientID:    a.OAuth.ClientID,
			})
		}
		return p.Print(entries)
	},
}

// ---- agent remove ----

var agentRemoveCmd = &cobra.Command{
	Use:     "remove <alias>",
	Aliases: []string{"rm", "delete"},
	Short:   "Remove a registered agent alias",
	Example: `  agc agent remove staging`,
	Args:    cobra.ExactArgs(1),
	RunE: func(_ *cobra.Command, args []string) error {
		alias := args[0]
		cfg, err := config.Load()
		if err != nil {
			return err
		}
		if _, ok := cfg.Agents[alias]; !ok {
			return fmt.Errorf("unknown alias %q", alias)
		}
		delete(cfg.Agents, alias)
		if cfg.CurrentAgent == alias {
			cfg.CurrentAgent = ""
			fmt.Printf("Removed active agent %q — run 'agc agent use <alias>' to set a new one\n", alias)
		} else {
			fmt.Printf("Removed agent %q\n", alias)
		}
		return config.Save(cfg)
	},
}

// ---- agent show ----

var agentShowCmd = &cobra.Command{
	Use:   "show [alias]",
	Short: "Show details for an agent alias (defaults to active)",
	Example: `  agc agent show
  agc agent show prod`,
	Args: cobra.MaximumNArgs(1),
	RunE: func(_ *cobra.Command, args []string) error {
		p := printer()
		cfg, err := config.Load()
		if err != nil {
			return err
		}

		alias := cfg.CurrentAgent
		if len(args) > 0 {
			alias = args[0]
		}
		if alias == "" {
			return fmt.Errorf("no active agent — run: agc agent use <alias>")
		}
		agent, ok := cfg.Agents[alias]
		if !ok {
			return fmt.Errorf("unknown alias %q", alias)
		}

		type view struct {
			Alias       string             `json:"alias"`
			URL         string             `json:"url"`
			Active      bool               `json:"active"`
			Transport   string             `json:"transport,omitempty"`
			Description string             `json:"description,omitempty"`
			OAuth       config.OAuthConfig `json:"oauth,omitempty"`
		}
		return p.Print(view{
			Alias:       alias,
			URL:         agent.URL,
			Active:      alias == cfg.CurrentAgent,
			Transport:   agent.Transport,
			Description: agent.Description,
			OAuth:       agent.OAuth,
		})
	},
}

// ---- agent update (set description / client-id / scopes) ----

var agentUpdateFlags struct {
	description string
	clientID    string
	scopes      []string
	url         string
	transport   string
}

var agentUpdateCmd = &cobra.Command{
	Use:   "update <alias>",
	Short: "Update settings for an existing agent alias",
	Example: `  agc agent update prod --description "Production v2"
  agc agent update local --client-id new-app-id
  agc agent update staging --url https://new-staging.example.com`,
	Args: cobra.ExactArgs(1),
	RunE: func(_ *cobra.Command, args []string) error {
		alias := args[0]
		cfg, err := config.Load()
		if err != nil {
			return err
		}
		agent, ok := cfg.Agents[alias]
		if !ok {
			return fmt.Errorf("unknown alias %q — register with: agc agent add %s <url>", alias, alias)
		}
		if agentUpdateFlags.url != "" {
			if err := validate.AgentURL(agentUpdateFlags.url); err != nil {
				return err
			}
			agent.URL = agentUpdateFlags.url
		}
		if agentUpdateFlags.description != "" {
			agent.Description = agentUpdateFlags.description
		}
		if agentUpdateFlags.clientID != "" {
			if err := validate.DangerousInput(agentUpdateFlags.clientID, "--client-id"); err != nil {
				return err
			}
			agent.OAuth.ClientID = agentUpdateFlags.clientID
		}
		if len(agentUpdateFlags.scopes) > 0 {
			agent.OAuth.Scopes = agentUpdateFlags.scopes
		}
		if agentUpdateFlags.transport != "" {
			agent.Transport = normalizeTransport(agentUpdateFlags.transport)
		}
		cfg.SetAgent(alias, agent)
		if err := config.Save(cfg); err != nil {
			return err
		}
		fmt.Printf("Updated agent %q\n", alias)
		return nil
	},
}

func init() {
	agentUpdateCmd.Flags().StringVar(&agentUpdateFlags.url, "url", "", "New URL for this agent")
	agentUpdateCmd.Flags().StringVar(&agentUpdateFlags.description, "description", "", "New description")
	agentUpdateCmd.Flags().StringVar(&agentUpdateFlags.clientID, "client-id", "", "New OAuth client ID")
	agentUpdateCmd.Flags().StringSliceVar(&agentUpdateFlags.scopes, "scopes", nil, "New OAuth scopes")
	agentUpdateCmd.Flags().StringVar(&agentUpdateFlags.transport, "transport", "", "New transport binding: JSONRPC or HTTP+JSON")
	agentCmd.AddCommand(agentUpdateCmd)
}

// suppress unused import warning for output (used in agentListCmd via p.Print)
