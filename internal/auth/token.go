// Package auth handles authentication flows and secure token storage for the a2a CLI.
package auth

import (
	"crypto/aes"
	"crypto/cipher"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/url"
	"os"
	"path/filepath"
	"time"

	"github.com/zalando/go-keyring"
)

const (
	keychainService    = "agc"
	keychainEncKeyUser = "encryption-key"
)

// StorageBackend selects where tokens are persisted.
// Controlled by the A2A_CLI_KEYRING_BACKEND environment variable.
type StorageBackend string

const (
	BackendKeyring StorageBackend = "keyring" // OS keyring (default)
	BackendFile    StorageBackend = "file"    // Encrypted file (headless/Docker)
)

// activeBackend reads A2A_CLI_KEYRING_BACKEND and returns the configured backend.
func activeBackend() StorageBackend {
	switch os.Getenv("AGC_KEYRING_BACKEND") {
	case "file":
		return BackendFile
	default:
		return BackendKeyring
	}
}

// TokenSet holds an OAuth token response persisted securely.
type TokenSet struct {
	AccessToken  string    `json:"access_token"`
	RefreshToken string    `json:"refresh_token,omitempty"`
	ExpiresAt    time.Time `json:"expires_at,omitempty"`
	TokenType    string    `json:"token_type,omitempty"`
	Scopes       []string  `json:"scopes,omitempty"`
}

// Valid returns true if the token is non-empty and has not expired (30s buffer).
func (t *TokenSet) Valid() bool {
	if t == nil || t.AccessToken == "" {
		return false
	}
	if t.ExpiresAt.IsZero() {
		return true // no expiry — assume valid
	}
	return time.Now().Add(30 * time.Second).Before(t.ExpiresAt)
}

// CanRefresh returns true if a refresh token is present.
func (t *TokenSet) CanRefresh() bool {
	return t != nil && t.RefreshToken != ""
}

// SaveToken persists a TokenSet. Uses OS keyring first; falls back to an
// AES-256-GCM encrypted file when the keyring is unavailable.
func SaveToken(agentURL string, token *TokenSet) error {
	key, err := keychainKey(agentURL)
	if err != nil {
		return err
	}
	data, err := json.Marshal(token)
	if err != nil {
		return fmt.Errorf("marshal token: %w", err)
	}
	payload := string(data)

	if activeBackend() == BackendFile {
		return saveTokenFile(key, payload)
	}

	if err := keyring.Set(keychainService, key, payload); err != nil {
		// Keyring unavailable — fall back to encrypted file and warn.
		fmt.Fprintf(os.Stderr, "warning: keyring unavailable (%v), using encrypted file storage\n", err)
		return saveTokenFile(key, payload)
	}
	return nil
}

// LoadToken retrieves a stored TokenSet. Returns nil, nil if not found.
func LoadToken(agentURL string) (*TokenSet, error) {
	key, err := keychainKey(agentURL)
	if err != nil {
		return nil, err
	}

	var payload string

	if activeBackend() == BackendKeyring {
		payload, err = keyring.Get(keychainService, key)
		if err != nil && !errors.Is(err, keyring.ErrNotFound) {
			// Keyring error — try file fallback silently
			payload = ""
		}
	}

	if payload == "" {
		payload, err = loadTokenFile(key)
		if err != nil || payload == "" {
			return nil, nil //nolint:nilerr
		}
	}

	var token TokenSet
	if err := json.Unmarshal([]byte(payload), &token); err != nil {
		return nil, fmt.Errorf("parse stored token: %w", err)
	}
	return &token, nil
}

// DeleteToken removes the token from the keyring and the encrypted file.
func DeleteToken(agentURL string) error {
	key, err := keychainKey(agentURL)
	if err != nil {
		return err
	}

	// Remove from keyring (ignore not-found).
	if krErr := keyring.Delete(keychainService, key); krErr != nil && !errors.Is(krErr, keyring.ErrNotFound) {
		fmt.Fprintf(os.Stderr, "warning: could not delete from keyring: %v\n", krErr)
	}

	// Remove encrypted file if present.
	path, pathErr := tokenFilePath(key)
	if pathErr == nil {
		if rmErr := os.Remove(path); rmErr != nil && !errors.Is(rmErr, os.ErrNotExist) {
			return fmt.Errorf("delete token file: %w", rmErr)
		}
	}
	return nil
}

// --------------------------------------------------------------------------
// Encrypted-file backend
// --------------------------------------------------------------------------

func saveTokenFile(key, payload string) error {
	encKey, err := getOrCreateEncryptionKey()
	if err != nil {
		return fmt.Errorf("encryption key: %w", err)
	}
	ciphertext, err := aesGCMEncrypt(encKey, []byte(payload))
	if err != nil {
		return fmt.Errorf("encrypt token: %w", err)
	}

	path, err := tokenFilePath(key)
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return fmt.Errorf("create token dir: %w", err)
	}
	encoded := base64.StdEncoding.EncodeToString(ciphertext)
	return os.WriteFile(path, []byte(encoded), 0o600)
}

func loadTokenFile(key string) (string, error) {
	path, err := tokenFilePath(key)
	if err != nil {
		return "", err
	}
	encoded, err := os.ReadFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return "", nil
	}
	if err != nil {
		return "", err
	}
	ciphertext, err := base64.StdEncoding.DecodeString(string(encoded))
	if err != nil {
		return "", fmt.Errorf("decode token file: %w", err)
	}
	encKey, err := getOrCreateEncryptionKey()
	if err != nil {
		return "", fmt.Errorf("encryption key: %w", err)
	}
	plaintext, err := aesGCMDecrypt(encKey, ciphertext)
	if err != nil {
		return "", fmt.Errorf("decrypt token file: %w", err)
	}
	return string(plaintext), nil
}

func tokenFilePath(key string) (string, error) {
	base, err := os.UserConfigDir()
	if err != nil {
		return "", fmt.Errorf("config dir: %w", err)
	}
	// Sanitize key to be a valid filename.
	safe := url.PathEscape(key)
	return filepath.Join(base, "a2a", "tokens", safe+".enc"), nil
}

// --------------------------------------------------------------------------
// Encryption key management
// --------------------------------------------------------------------------

// getOrCreateEncryptionKey returns the AES-256 key used for file-based token
// storage. Tries the OS keyring first; generates and persists a new key if absent.
func getOrCreateEncryptionKey() ([32]byte, error) {
	var key [32]byte

	// Try keyring first.
	if b64, err := keyring.Get(keychainService, keychainEncKeyUser); err == nil {
		decoded, decErr := base64.StdEncoding.DecodeString(b64)
		if decErr == nil && len(decoded) == 32 {
			copy(key[:], decoded)
			return key, nil
		}
	}

	// Try local key file.
	if raw, err := readEncKeyFile(); err == nil && len(raw) == 32 {
		copy(key[:], raw)
		// Best-effort: sync to keyring.
		_ = keyring.Set(keychainService, keychainEncKeyUser, base64.StdEncoding.EncodeToString(raw))
		return key, nil
	}

	// Generate a new key and persist it.
	if _, err := io.ReadFull(rand.Reader, key[:]); err != nil {
		return key, fmt.Errorf("generate encryption key: %w", err)
	}
	b64 := base64.StdEncoding.EncodeToString(key[:])
	_ = keyring.Set(keychainService, keychainEncKeyUser, b64)
	_ = writeEncKeyFile(key[:])
	return key, nil
}

func encKeyFilePath() (string, error) {
	base, err := os.UserConfigDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(base, "a2a", ".encryption_key"), nil
}

func readEncKeyFile() ([]byte, error) {
	path, err := encKeyFilePath()
	if err != nil {
		return nil, err
	}
	b64, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	return base64.StdEncoding.DecodeString(string(b64))
}

func writeEncKeyFile(key []byte) error {
	path, err := encKeyFilePath()
	if err != nil {
		return err
	}
	if err := os.MkdirAll(filepath.Dir(path), 0o700); err != nil {
		return err
	}
	return os.WriteFile(path, []byte(base64.StdEncoding.EncodeToString(key)), 0o600)
}

// --------------------------------------------------------------------------
// AES-256-GCM helpers
// --------------------------------------------------------------------------

func aesGCMEncrypt(key [32]byte, plaintext []byte) ([]byte, error) {
	block, err := aes.NewCipher(key[:])
	if err != nil {
		return nil, err
	}
	gcm, err := cipher.NewGCM(block)
	if err != nil {
		return nil, err
	}
	nonce := make([]byte, gcm.NonceSize())
	if _, err := io.ReadFull(rand.Reader, nonce); err != nil {
		return nil, err
	}
	// Prepend nonce to ciphertext.
	return gcm.Seal(nonce, nonce, plaintext, nil), nil
}

func aesGCMDecrypt(key [32]byte, data []byte) ([]byte, error) {
	block, err := aes.NewCipher(key[:])
	if err != nil {
		return nil, err
	}
	gcm, err := cipher.NewGCM(block)
	if err != nil {
		return nil, err
	}
	nonceSize := gcm.NonceSize()
	if len(data) < nonceSize {
		return nil, fmt.Errorf("encrypted data too short")
	}
	nonce, ciphertext := data[:nonceSize], data[nonceSize:]
	return gcm.Open(nil, nonce, ciphertext, nil)
}

// --------------------------------------------------------------------------
// Key derivation utilities
// --------------------------------------------------------------------------

// keychainKey derives a stable keychain username from an agent URL.
// Uses <host>[:<port>] so multiple agents get separate entries.
func keychainKey(agentURL string) (string, error) {
	u, err := url.Parse(agentURL)
	if err != nil {
		return "", fmt.Errorf("invalid agent URL %q: %w", agentURL, err)
	}
	key := u.Hostname()
	if port := u.Port(); port != "" {
		key += ":" + port
	}
	return key, nil
}

// --------------------------------------------------------------------------
// Client secret storage
