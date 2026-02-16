package ota

import (
	"crypto/ed25519"
	"encoding/base64"
	"os"
	"path/filepath"
	"testing"

	"bros/internal/native/common"
)

func TestParseSparklePrivateKey_Base64Seed32(t *testing.T) {
	seed := make([]byte, ed25519.SeedSize)
	for i := range seed {
		seed[i] = byte(i)
	}

	encoded := base64.StdEncoding.EncodeToString(seed)
	parsed, err := parseSparklePrivateKey(encoded)
	if err != nil {
		t.Fatalf("parseSparklePrivateKey returned error: %v", err)
	}

	expected := ed25519.NewKeyFromSeed(seed)
	if string(parsed) != string(expected) {
		t.Fatalf("parsed key mismatch")
	}
}

func TestParseSparklePrivateKey_Base64Sparkle64(t *testing.T) {
	seed := make([]byte, ed25519.SeedSize)
	for i := range seed {
		seed[i] = byte(i + 1)
	}

	privateKey := ed25519.NewKeyFromSeed(seed)
	combined := make([]byte, 0, ed25519.PrivateKeySize)
	combined = append(combined, seed...)
	combined = append(combined, privateKey.Public().(ed25519.PublicKey)...)

	encoded := base64.StdEncoding.EncodeToString(combined)
	parsed, err := parseSparklePrivateKey(encoded)
	if err != nil {
		t.Fatalf("parseSparklePrivateKey returned error: %v", err)
	}

	if string(parsed) != string(privateKey) {
		t.Fatalf("parsed key mismatch")
	}
}

func TestParseSparklePrivateKey_InvalidLength(t *testing.T) {
	encoded := base64.StdEncoding.EncodeToString([]byte("too-short"))
	if _, err := parseSparklePrivateKey(encoded); err == nil {
		t.Fatalf("expected invalid length error")
	}
}

func TestSignWithSparkle(t *testing.T) {
	seed := make([]byte, ed25519.SeedSize)
	for i := range seed {
		seed[i] = byte(i + 2)
	}
	env := common.EnvConfig{SparklePrivateKey: base64.StdEncoding.EncodeToString(seed)}

	dir := t.TempDir()
	filePath := filepath.Join(dir, "artifact.zip")
	if err := os.WriteFile(filePath, []byte("hello world"), 0o644); err != nil {
		t.Fatalf("write test artifact: %v", err)
	}

	signature, length, err := signWithSparkle(filePath, env)
	if err != nil {
		t.Fatalf("signWithSparkle returned error: %v", err)
	}
	if signature == "" {
		t.Fatalf("signature should not be empty")
	}
	if length != int64(len("hello world")) {
		t.Fatalf("unexpected length: %d", length)
	}
}
