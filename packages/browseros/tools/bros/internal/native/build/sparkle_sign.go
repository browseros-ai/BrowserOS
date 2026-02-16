package build

import (
	"crypto/ed25519"
	"encoding/base64"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

func runSparkleSign(ctx *Context) error {
	if !ctx.Env.HasSparkleKey() {
		return fmt.Errorf("SPARKLE_PRIVATE_KEY environment variable not set")
	}

	distDir := ctx.DistDir()
	if !dirExists(distDir) {
		return nil
	}

	entries, err := os.ReadDir(distDir)
	if err != nil {
		return fmt.Errorf("reading dist dir: %w", err)
	}

	signed := 0
	for _, entry := range entries {
		if entry.IsDir() {
			continue
		}
		if strings.ToLower(filepath.Ext(entry.Name())) != ".dmg" {
			continue
		}

		fullPath := filepath.Join(distDir, entry.Name())
		sig, length, err := signSparkleFile(fullPath, ctx.Env.SparklePrivateKey)
		if err != nil {
			return fmt.Errorf("signing %s: %w", entry.Name(), err)
		}
		ctx.SparkleSignatures[entry.Name()] = SparkleSignature{
			Signature: sig,
			Length:    length,
		}
		signed++
	}

	if signed == 0 {
		return nil
	}

	return nil
}

func signSparkleFile(filePath string, privateKeyValue string) (string, int64, error) {
	privateKey, err := parseSparklePrivateKey(privateKeyValue)
	if err != nil {
		return "", 0, err
	}

	fileData, err := os.ReadFile(filePath)
	if err != nil {
		return "", 0, fmt.Errorf("reading file: %w", err)
	}

	signature := ed25519.Sign(privateKey, fileData)
	return base64.StdEncoding.EncodeToString(signature), int64(len(fileData)), nil
}

func parseSparklePrivateKey(keyData string) (ed25519.PrivateKey, error) {
	keyData = strings.TrimSpace(keyData)
	if keyData == "" {
		return nil, fmt.Errorf("SPARKLE_PRIVATE_KEY is empty")
	}

	keyBytes, decoded := decodePossiblyBase64(keyData)
	if !decoded {
		keyBytes = []byte(keyData)
	}

	switch len(keyBytes) {
	case ed25519.SeedSize:
		return ed25519.NewKeyFromSeed(keyBytes), nil
	case ed25519.PrivateKeySize:
		return ed25519.NewKeyFromSeed(keyBytes[:ed25519.SeedSize]), nil
	default:
		return nil, fmt.Errorf("invalid Sparkle key length: %d bytes (expected 32 or 64)", len(keyBytes))
	}
}

func decodePossiblyBase64(input string) ([]byte, bool) {
	if decoded, err := base64.StdEncoding.DecodeString(input); err == nil {
		return decoded, true
	}

	if decoded, err := base64.RawStdEncoding.DecodeString(input); err == nil {
		return decoded, true
	}

	padded := input
	if rem := len(input) % 4; rem != 0 {
		padded = input + strings.Repeat("=", 4-rem)
		if decoded, err := base64.StdEncoding.DecodeString(padded); err == nil {
			return decoded, true
		}
	}

	return nil, false
}
