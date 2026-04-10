// Package main is the entry point for the agc (Agent CLI) tool.
package main

import (
	"errors"
	"os"

	"genai.stargate.toyota/agc/internal/cli"
)

// Exit codes — documented in the agc-shared SKILL.md so AI tools know how to handle them.
const (
	exitOK           = 0
	exitError        = 1
	exitInputRequired = 2 // agent paused; needs a reply via --task-id
	exitAuthRequired  = 3 // agent needs OAuth; human must authenticate
)

func main() {
	err := cli.ExecuteErr()
	if err == nil {
		os.Exit(exitOK)
	}
	if errors.Is(err, cli.ErrInputRequired) {
		os.Exit(exitInputRequired)
	}
	if errors.Is(err, cli.ErrAuthRequired) {
		os.Exit(exitAuthRequired)
	}
	os.Exit(exitError)
}
