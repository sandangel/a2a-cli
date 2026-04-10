package cli

import (
	"context"
	"fmt"
	"strings"
	"sync"

	"github.com/a2aproject/a2a-go/v2/a2a"
	"github.com/spf13/cobra"
	"genai.stargate.toyota/agc/internal/output"
)

var cardCmd = &cobra.Command{
	Use:   "card",
	Short: "Show agent card — capabilities, skills, auth",
	Example: `  agc card
  agc --agent prod card
  agc --all card
  agc --json card | jq .card.capabilities`,
	RunE: runCard,
}

func runCard(cmd *cobra.Command, _ []string) error {
	p := printer()
	targets, err := resolveTargets()
	if err != nil {
		p.PrintError(err)
		return err
	}

	ctx, cancel := cmdTimeout(cmd)
	defer cancel()

	if len(targets) == 1 {
		return fetchAndPrintCard(ctx, p, targets[0], false)
	}

	// Parallel fetch.
	type cardResult struct {
		target AgentTarget
		card   *a2a.AgentCard
		err    error
	}
	ch := make(chan cardResult, len(targets))
	var wg sync.WaitGroup
	for _, t := range targets {
		wg.Add(1)
		go func(t AgentTarget) {
			defer wg.Done()
			card, err := compatCardResolver.Resolve(ctx, t.URL)
			ch <- cardResult{target: t, card: card, err: err}
		}(t)
	}
	go func() { wg.Wait(); close(ch) }()

	var anyError bool
	for r := range ch {
		if r.err != nil {
			p.PrintError(fmt.Errorf("[%s] %w", r.target.Label(), r.err))
			anyError = true
			continue
		}
		if printErr := fetchAndPrintCard(ctx, p, r.target, true); printErr != nil {
			return printErr
		}
	}
	if anyError {
		return fmt.Errorf("one or more card fetches failed")
	}
	return nil
}

func fetchAndPrintCard(ctx context.Context, p *output.Printer, t AgentTarget, multiAgent bool) error {
	card, err := compatCardResolver.Resolve(ctx, t.URL)
	if err != nil {
		p.PrintError(fmt.Errorf("fetch card: %w", err))
		return err
	}

	if false { // raw format removed
		return p.Print(card)
	}

	if flags.format == "json" {
		if multiAgent {
			// Multi-agent: wrap with identity so caller knows which card came from which agent.
			type cardJSON struct {
				Agent    string         `json:"agent"`
				AgentURL string         `json:"agent_url"`
				Card     *a2a.AgentCard `json:"card"`
			}
			return p.PrintJSON(cardJSON{Agent: t.Label(), AgentURL: t.URL, Card: card})
		}
		// Single-agent: return raw AgentCard so --fields paths work directly.
		// Use agc schema card to understand the structure.
		return p.Print(card)
	}

	// Human output.
	if multiAgent {
		header := fmt.Sprintf("── %s (%s) ", t.Label(), t.URL)
		if len(header) < 60 {
			header += strings.Repeat("─", 60-len(header))
		}
		p.PrintLine(header)
	}
	printCardHuman(p, card)
	if multiAgent {
		p.PrintLine("")
	}
	return nil
}

func printCardHuman(p *output.Printer, card *a2a.AgentCard) {
	p.PrintLine(fmt.Sprintf("%-12s %s", "Name:", card.Name))
	if card.Version != "" {
		p.PrintLine(fmt.Sprintf("%-12s %s", "Version:", card.Version))
	}
	p.PrintLine(fmt.Sprintf("%-12s %s", "Description:", card.Description))
	if card.Provider != nil {
		p.PrintLine(fmt.Sprintf("%-12s %s  %s", "Provider:", card.Provider.Org, card.Provider.URL))
	}

	caps := []string{}
	if card.Capabilities.Streaming {
		caps = append(caps, "streaming")
	}
	if card.Capabilities.PushNotifications {
		caps = append(caps, "push-notifications")
	}
	if card.Capabilities.ExtendedAgentCard {
		caps = append(caps, "extended-card")
	}
	if len(caps) > 0 {
		p.PrintLine(fmt.Sprintf("%-12s %s", "Capabilities:", strings.Join(caps, ", ")))
	}

	if len(card.SecuritySchemes) > 0 {
		p.PrintLine("\nAuthentication:")
		for name, scheme := range card.SecuritySchemes {
			switch s := scheme.(type) {
			case a2a.OAuth2SecurityScheme:
				flowName := oauthFlowName(s.Flows)
				p.PrintLine(fmt.Sprintf("  %-18s OAuth2 (%s)", name, flowName))
			case a2a.HTTPAuthSecurityScheme:
				p.PrintLine(fmt.Sprintf("  %-18s HTTP Bearer (%s)", name, s.Scheme))
			case a2a.APIKeySecurityScheme:
				p.PrintLine(fmt.Sprintf("  %-18s API Key: %s in %s", name, s.Name, s.Location))
			case a2a.OpenIDConnectSecurityScheme:
				p.PrintLine(fmt.Sprintf("  %-18s OpenID Connect", name))
			}
		}
	}

	if len(card.Skills) > 0 {
		p.PrintLine(fmt.Sprintf("\nSkills (%d):", len(card.Skills)))
		headers := []string{"ID", "NAME", "DESCRIPTION"}
		var rows [][]string
		for _, skill := range card.Skills {
			rows = append(rows, []string{skill.ID, skill.Name, output.Truncate(skill.Description, 60)})
		}
		p.PrintTable(headers, rows)
	}
}

func oauthFlowName(flows a2a.OAuthFlows) string {
	switch flows.(type) {
	case a2a.DeviceCodeOAuthFlow:
		return "deviceCode"
	case a2a.AuthorizationCodeOAuthFlow:
		return "authorizationCode"
	case a2a.ClientCredentialsOAuthFlow:
		return "clientCredentials"
	default:
		return "other"
	}
}
