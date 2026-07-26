package pipeline

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"

	"browseros-dogfood/config"
)

// WriteProductionEnvFile writes the merged production env file at the agent root.
func WriteProductionEnvFile(agentRoot string, cfg config.Config) error {
	cfg.FillProductionEnvDefaults()
	values := make(map[string]string, len(cfg.ProductionEnv.CLI)+len(cfg.ProductionEnv.Server))
	for key, value := range cfg.ProductionEnv.CLI {
		values[key] = value
	}
	for key, value := range cfg.ProductionEnv.Server {
		if value != "" {
			values[key] = value
			continue
		}
		if _, ok := values[key]; !ok {
			values[key] = value
		}
	}
	return writeEnvFile(filepath.Join(agentRoot, ".env.production"), values)
}

func writeEnvFile(path string, values map[string]string) error {
	keys := make([]string, 0, len(values))
	for key := range values {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	var out bytes.Buffer
	for _, key := range keys {
		line, err := formatEnvLine(key, values[key])
		if err != nil {
			return err
		}
		out.WriteString(line)
		out.WriteByte('\n')
	}
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return err
	}
	return os.WriteFile(path, out.Bytes(), 0600)
}

func formatEnvLine(key string, value string) (string, error) {
	if key == "" || strings.ContainsAny(key, " \t\r\n=") {
		return "", fmt.Errorf("invalid env key %q", key)
	}
	if strings.ContainsAny(value, "\r\n") {
		return "", fmt.Errorf("env value for %s must not contain newlines", key)
	}
	if strings.ContainsAny(value, " \t#'\"=") {
		value = strconv.Quote(value)
	}
	return fmt.Sprintf("%s=%s", key, value), nil
}
