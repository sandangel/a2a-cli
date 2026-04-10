// Package config manages persistent configuration for the agc CLI.
package config

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"time"

	"gopkg.in/yaml.v3"
)

const (
	configFileName = "config.yaml"
	configDirName  = "agc"
	defaultProfile = "default"
)

// Config is the root configuration structure stored in the config file.
type Config struct {
	// CurrentAgent is the name of the currently active agent alias.
	// Takes precedence over CurrentProfile.AgentURL when set.
	CurrentAgent string `yaml:"current_agent,omitempty"`

	// Agents is the named agent registry. Keys are short alias names.
	Agents map[string]Agent `yaml:"agents,omitempty"`

	// CurrentProfile is the name of the active profile (for output prefs).
	CurrentProfile string `yaml:"current_profile,omitempty"`

	// Profiles holds per-profile output preferences.
	Profiles map[string]Profile `yaml:"profiles,omitempty"`
}

// Agent represents a named A2A agent endpoint with its connection settings.
type Agent struct {
	// URL is the base URL of the agent.
	URL string `yaml:"url"`

	// Description is an optional human-readable label.
	Description string `yaml:"description,omitempty"`

	// Transport sets the preferred protocol binding for this agent.
	// Values: "jsonrpc" (default), "rest"
	// The SDK auto-negotiates from the agent card when unset.
	Transport string `yaml:"transport,omitempty"`

	// OAuth holds OAuth client config specific to this agent.
	OAuth OAuthConfig `yaml:"oauth,omitempty"`
}

// Profile holds output and UX preferences (not connection settings).
type Profile struct {
	// AgentURL is a fallback agent URL for profiles that predate the agent registry.
	AgentURL string `yaml:"agent_url,omitempty"`

	Format  string        `yaml:"format,omitempty"`
	Timeout time.Duration `yaml:"timeout,omitempty"`
	OAuth   OAuthConfig   `yaml:"oauth,omitempty"`
}

// OAuthConfig holds OAuth client configuration (non-secret parts).
// The client_secret is stored in the system keychain, not here.
type OAuthConfig struct {
	ClientID string   `yaml:"client_id,omitempty"`
	Scopes   []string `yaml:"scopes,omitempty"`
}

// Load reads the config file. If the file does not exist a default empty
// config is returned without error.
func Load() (*Config, error) {
	path, err := FilePath()
	if err != nil {
		return nil, err
	}

	data, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return defaultConfig(), nil
	}
	if err != nil {
		return nil, fmt.Errorf("read config: %w", err)
	}

	var cfg Config
	if err := yaml.Unmarshal(data, &cfg); err != nil {
		return nil, fmt.Errorf("parse config: %w", err)
	}
	if cfg.CurrentProfile == "" {
		cfg.CurrentProfile = defaultProfile
	}
	if cfg.Profiles == nil {
		cfg.Profiles = make(map[string]Profile)
	}
	if cfg.Agents == nil {
		cfg.Agents = make(map[string]Agent)
	}
	return &cfg, nil
}

// Save writes the config to the config file, creating it and its parent
// directory if necessary.
func Save(cfg *Config) error {
	path, err := FilePath()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return fmt.Errorf("create config dir: %w", err)
	}
	data, err := yaml.Marshal(cfg)
	if err != nil {
		return fmt.Errorf("marshal config: %w", err)
	}
	if err := os.WriteFile(path, data, 0o600); err != nil {
		return fmt.Errorf("write config: %w", err)
	}
	return nil
}

// CurrentProfileData returns the active profile. Falls back to empty Profile.
func (c *Config) CurrentProfileData() Profile {
	if p, ok := c.Profiles[c.CurrentProfile]; ok {
		return p
	}
	return Profile{}
}

// ResolveAgent resolves a name-or-URL string:
//   - If it matches an agent alias in c.Agents → returns that Agent
//   - If it looks like a URL (http/https) → returns a synthetic Agent with just the URL
//   - Otherwise returns false
func (c *Config) ResolveAgent(nameOrURL string) (Agent, bool) {
	// Named alias takes priority.
	if a, ok := c.Agents[nameOrURL]; ok {
		return a, true
	}
	// Raw URL.
	if IsURL(nameOrURL) {
		return Agent{URL: nameOrURL}, true
	}
	return Agent{}, false
}

// ActiveAgent returns the agent that should be used for the current session:
// c.CurrentAgent (alias lookup) → current profile AgentURL fallback.
func (c *Config) ActiveAgent() (Agent, bool) {
	if c.CurrentAgent != "" {
		if a, ok := c.Agents[c.CurrentAgent]; ok {
			return a, true
		}
	}
	// Legacy fallback: profile agent_url
	if url := c.CurrentProfileData().AgentURL; url != "" {
		return Agent{URL: url}, true
	}
	return Agent{}, false
}

// ActiveOAuth returns the OAuth config for the active agent, merging
// agent-specific config with the current profile fallback.
func (c *Config) ActiveOAuth(agentName string) OAuthConfig {
	// Agent-specific config wins.
	if agentName != "" {
		if a, ok := c.Agents[agentName]; ok && a.OAuth.ClientID != "" {
			return a.OAuth
		}
	}
	return c.CurrentProfileData().OAuth
}

// SetAgent upserts an agent in the registry.
func (c *Config) SetAgent(name string, a Agent) {
	if c.Agents == nil {
		c.Agents = make(map[string]Agent)
	}
	c.Agents[name] = a
}

// SetProfile upserts a profile.
func (c *Config) SetProfile(name string, p Profile) {
	if c.Profiles == nil {
		c.Profiles = make(map[string]Profile)
	}
	c.Profiles[name] = p
}

// FilePath returns the absolute path to the config file.
func FilePath() (string, error) {
	dir, err := Dir()
	if err != nil {
		return "", err
	}
	return filepath.Join(dir, configFileName), nil
}

// Dir returns the configuration directory (e.g. ~/.config/agc).
func Dir() (string, error) {
	base, err := os.UserConfigDir()
	if err != nil {
		return "", fmt.Errorf("locate config dir: %w", err)
	}
	return filepath.Join(base, configDirName), nil
}

// BuildEnv is set at build time via -ldflags to select the target environment.
// Accepted values: "dev", "stg", "" / "prod" (default).
// Example: go build -ldflags "-X genai.stargate.toyota/agc/internal/config.BuildEnv=dev"
var BuildEnv string

// roverBaseHost returns the hostname for the Rover agent based on BuildEnv.
func roverBaseHost() string {
	switch BuildEnv {
	case "dev":
		return "dev.genai.stargate.toyota"
	case "stg":
		return "stg.genai.stargate.toyota"
	default: // "prod" or empty
		return "genai.stargate.toyota"
	}
}

// DefaultRoverAgentURL is the URL of the Rover A2A agent for the current build environment.
func DefaultRoverAgentURL() string {
	return "https://" + roverBaseHost() + "/a2a/rover-agent"
}

// defaultRoverClientID returns the CIMD client_id URL for the current build environment.
func defaultRoverClientID() string {
	return "https://" + roverBaseHost() + "/a2a/agc/.well-known/client-metadata.json"
}

func defaultConfig() *Config {
	return &Config{
		CurrentAgent:   "rover",
		CurrentProfile: defaultProfile,
		Profiles:       make(map[string]Profile),
		Agents: map[string]Agent{
			"rover": {
				URL:         DefaultRoverAgentURL(),
				Description: "Rover Agent",
				OAuth: OAuthConfig{
					ClientID: defaultRoverClientID(),
				},
			},
		},
	}
}

// IsURL returns true if s looks like an http/https URL.
func IsURL(s string) bool {
	return len(s) > 7 && (s[:7] == "http://" || s[:8] == "https://")
}
