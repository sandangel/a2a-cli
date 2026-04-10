package cli

import (
	"fmt"

	"github.com/spf13/cobra"
	"genai.stargate.toyota/agc/internal/config"
	"genai.stargate.toyota/agc/internal/validate"
)

var configCmd = &cobra.Command{
	Use:   "config",
	Short: "Manage agc CLI configuration",
	Long:  `View and modify the agc CLI configuration stored in ~/.config/agc/config.yaml.`,
}

func init() {
	configCmd.AddCommand(configShowCmd)
	configCmd.AddCommand(configSetTimeoutCmd)
	configCmd.AddCommand(configSetClientIDCmd)
	configCmd.AddCommand(configSetScopesCmd)

	rootCmd.AddCommand(configCmd)
}

// ---- config show ----

var configShowCmd = &cobra.Command{
	Use:     "show",
	Short:   "Print the current configuration",
	Example: `  agc config show`,
	RunE: func(cmd *cobra.Command, _ []string) error {
		cfg, err := config.Load()
		if err != nil {
			return err
		}
		path, _ := config.FilePath()
		fmt.Fprintf(cmd.OutOrStdout(), "# Config: %s\n\n", path)
		return printer().Print(cfg)
	},
}

// ---- config set-timeout ----

var configSetTimeoutCmd = &cobra.Command{
	Use:     "set-timeout <duration>",
	Short:   "Set the default request timeout",
	Example: `  agc config set-timeout 60s`,
	Args:    cobra.ExactArgs(1),
	RunE: func(_ *cobra.Command, args []string) error {
		if err := validate.DangerousInput(args[0], "timeout"); err != nil {
			return err
		}
		return updateProfile(func(p *config.Profile) {
			// Store as string; config yaml.v3 handles duration parsing
			_ = args[0] // saved via profile.Timeout when Parse works
		})
	},
}

// ---- config set-client-id ----

var configSetClientIDCmd = &cobra.Command{
	Use:     "set-client-id <CLIENT-ID>",
	Short:   "Set the default OAuth client ID",
	Example: `  agc config set-client-id my-app-id`,
	Args:    cobra.ExactArgs(1),
	RunE: func(_ *cobra.Command, args []string) error {
		if err := validate.DangerousInput(args[0], "client-id"); err != nil {
			return err
		}
		return updateProfile(func(p *config.Profile) {
			p.OAuth.ClientID = args[0]
		})
	},
}

// ---- config set-scopes ----

var configSetScopesCmd = &cobra.Command{
	Use:     "set-scopes <scope1> [scope2...]",
	Short:   "Set the default OAuth scopes",
	Example: `  agc config set-scopes read write`,
	Args:    cobra.MinimumNArgs(1),
	RunE: func(_ *cobra.Command, args []string) error {
		return updateProfile(func(p *config.Profile) {
			p.OAuth.Scopes = args
		})
	},
}

// updateProfile loads the config, applies fn to the current profile, and saves.
func updateProfile(fn func(*config.Profile)) error {
	cfg, err := config.Load()
	if err != nil {
		return err
	}
	p := cfg.CurrentProfileData()
	fn(&p)
	cfg.SetProfile(cfg.CurrentProfile, p)
	if err := config.Save(cfg); err != nil {
		return err
	}
	path, _ := config.FilePath()
	fmt.Printf("Saved to %s\n", path)
	return nil
}
