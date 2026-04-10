package auth

// OAuth client metadata helpers for the agc CLI.
// All OAuth endpoint URLs (token, authorization) come from the A2A agent card's
// securitySchemes — no server-side discovery needed.

import (
	"errors"
	"fmt"
	"os"

	"github.com/zalando/go-keyring"
)

// --------------------------------------------------------------------------
// Client secret keychain storage
// --------------------------------------------------------------------------

// SaveClientSecret stores the OAuth client secret in the keychain.
// Never stored in the config file — too sensitive.
func SaveClientSecret(agentURL, secret string) error {
	key, err := keychainKey(agentURL)
	if err != nil {
		return err
	}
	if err := keyring.Set(keychainService, key+":client_secret", secret); err != nil {
		return fmt.Errorf("save client secret to keychain: %w", err)
	}
	return nil
}

// LoadClientSecret retrieves the client secret.
// Resolution order: AGC_CLIENT_SECRET env var → keychain.
func LoadClientSecret(agentURL string) (string, error) {
	if v := os.Getenv("AGC_CLIENT_SECRET"); v != "" {
		return v, nil
	}
	key, err := keychainKey(agentURL)
	if err != nil {
		return "", err
	}
	secret, err := keyring.Get(keychainService, key+":client_secret")
	if errors.Is(err, keyring.ErrNotFound) {
		return "", nil
	}
	if err != nil {
		return "", fmt.Errorf("read client secret from keychain: %w", err)
	}
	return secret, nil
}

// DeleteClientSecret removes the stored client secret.
func DeleteClientSecret(agentURL string) error {
	key, err := keychainKey(agentURL)
	if err != nil {
		return err
	}
	err = keyring.Delete(keychainService, key+":client_secret")
	if errors.Is(err, keyring.ErrNotFound) {
		return nil
	}
	return err
}
