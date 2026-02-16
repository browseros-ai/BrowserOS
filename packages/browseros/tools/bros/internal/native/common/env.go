package common

import (
	"bufio"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// EnvConfig mirrors the subset of BrowserOS build env used by native OTA commands.
type EnvConfig struct {
	MacOSCertificateName      string
	MacOSNotarizationAppleID  string
	MacOSNotarizationTeamID   string
	MacOSNotarizationPassword string
	MacOSKeychainPassword     string
	CodeSignToolPath          string
	CodeSignToolExe           string
	ESignerUsername           string
	ESignerPassword           string
	ESignerTOTPSecret         string
	ESignerCredentialID       string
	R2AccountID               string
	R2AccessKeyID             string
	R2SecretAccessKey         string
	R2Bucket                  string
	SparklePrivateKey         string
}

func LoadEnv(packagesDir string) EnvConfig {
	loadDotEnv(packagesDir)

	bucket := strings.TrimSpace(os.Getenv("R2_BUCKET"))
	if bucket == "" {
		bucket = "browseros"
	}

	return EnvConfig{
		MacOSCertificateName:      strings.TrimSpace(os.Getenv("MACOS_CERTIFICATE_NAME")),
		MacOSNotarizationAppleID:  strings.TrimSpace(os.Getenv("PROD_MACOS_NOTARIZATION_APPLE_ID")),
		MacOSNotarizationTeamID:   strings.TrimSpace(os.Getenv("PROD_MACOS_NOTARIZATION_TEAM_ID")),
		MacOSNotarizationPassword: strings.TrimSpace(os.Getenv("PROD_MACOS_NOTARIZATION_PWD")),
		MacOSKeychainPassword:     strings.TrimSpace(os.Getenv("MACOS_KEYCHAIN_PASSWORD")),
		CodeSignToolPath:          strings.TrimSpace(os.Getenv("CODE_SIGN_TOOL_PATH")),
		CodeSignToolExe:           strings.TrimSpace(os.Getenv("CODE_SIGN_TOOL_EXE")),
		ESignerUsername:           strings.TrimSpace(os.Getenv("ESIGNER_USERNAME")),
		ESignerPassword:           strings.TrimSpace(os.Getenv("ESIGNER_PASSWORD")),
		ESignerTOTPSecret:         strings.TrimSpace(os.Getenv("ESIGNER_TOTP_SECRET")),
		ESignerCredentialID:       strings.TrimSpace(os.Getenv("ESIGNER_CREDENTIAL_ID")),
		R2AccountID:               strings.TrimSpace(os.Getenv("R2_ACCOUNT_ID")),
		R2AccessKeyID:             strings.TrimSpace(os.Getenv("R2_ACCESS_KEY_ID")),
		R2SecretAccessKey:         strings.TrimSpace(os.Getenv("R2_SECRET_ACCESS_KEY")),
		R2Bucket:                  bucket,
		SparklePrivateKey:         strings.TrimSpace(os.Getenv("SPARKLE_PRIVATE_KEY")),
	}
}

func (e EnvConfig) HasSparkleKey() bool {
	return e.SparklePrivateKey != ""
}

func (e EnvConfig) HasR2Config() bool {
	return e.R2AccountID != "" && e.R2AccessKeyID != "" && e.R2SecretAccessKey != ""
}

func (e EnvConfig) R2EndpointURL() string {
	if e.R2AccountID == "" {
		return ""
	}
	return fmt.Sprintf("https://%s.r2.cloudflarestorage.com", e.R2AccountID)
}

func loadDotEnv(packagesDir string) {
	if packagesDir == "" {
		return
	}

	repoRoot := filepath.Clean(filepath.Join(packagesDir, "..", ".."))
	candidates := []string{
		filepath.Join(packagesDir, ".env"),
		filepath.Join(repoRoot, ".env"),
	}

	for _, candidate := range candidates {
		if !isExistingRegularFile(candidate) {
			continue
		}
		_ = loadDotEnvFile(candidate)
		return
	}
}

func loadDotEnvFile(path string) error {
	f, err := os.Open(path)
	if err != nil {
		return err
	}
	defer f.Close()

	s := bufio.NewScanner(f)
	for s.Scan() {
		line := strings.TrimSpace(s.Text())
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}

		if strings.HasPrefix(line, "export ") {
			line = strings.TrimSpace(strings.TrimPrefix(line, "export "))
		}

		key, value, ok := strings.Cut(line, "=")
		if !ok {
			continue
		}

		key = strings.TrimSpace(key)
		if key == "" {
			continue
		}

		if _, exists := os.LookupEnv(key); exists {
			continue
		}

		value = strings.TrimSpace(value)
		value = stripOuterQuotes(value)
		_ = os.Setenv(key, value)
	}

	return s.Err()
}

func stripOuterQuotes(s string) string {
	if len(s) < 2 {
		return s
	}

	if (s[0] == '"' && s[len(s)-1] == '"') || (s[0] == '\'' && s[len(s)-1] == '\'') {
		return s[1 : len(s)-1]
	}
	return s
}

func isExistingRegularFile(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.Mode().IsRegular()
}
