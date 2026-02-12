package config

import (
	"fmt"
	"os"
	"path/filepath"
)

const BrosDirName = ".bros"

// Context holds everything needed for an operation.
type Context struct {
	Config          *Config
	State           *State
	ChromiumDir     string // Absolute path to chromium checkout (parent of .bros/)
	BrosDir         string // Absolute path to .bros/
	PatchesRepo     string // Absolute path to patches repo root
	PatchesDir      string // Absolute path to chromium_patches/
	BaseCommit      string
	ChromiumVersion string
}

// FindBrosDir walks up from cwd to find the nearest .bros/ directory.
func FindBrosDir() (string, error) {
	dir, err := os.Getwd()
	if err != nil {
		return "", fmt.Errorf("getting cwd: %w", err)
	}

	for {
		candidate := filepath.Join(dir, BrosDirName)
		if info, err := os.Stat(candidate); err == nil && info.IsDir() {
			return dir, nil
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			break
		}
		dir = parent
	}

	return "", fmt.Errorf("not a bros checkout (no .bros/ found in any parent directory)")
}

// LoadContext loads config, state, and patches repo info.
func LoadContext() (*Context, error) {
	chromiumDir, err := FindBrosDir()
	if err != nil {
		return nil, err
	}
	brosDir := filepath.Join(chromiumDir, BrosDirName)

	cfg, err := ReadConfig(brosDir)
	if err != nil {
		return nil, fmt.Errorf("loading config: %w", err)
	}

	state, err := ReadState(brosDir)
	if err != nil {
		return nil, fmt.Errorf("loading state: %w", err)
	}

	patchesRepo := cfg.PatchesRepo
	if !filepath.IsAbs(patchesRepo) {
		patchesRepo = filepath.Join(chromiumDir, patchesRepo)
	}

	patchesDir := filepath.Join(patchesRepo, "chromium_patches")
	if _, err := os.Stat(patchesDir); err != nil {
		return nil, fmt.Errorf("patches directory not found: %s", patchesDir)
	}

	baseCommit, err := ReadBaseCommit(patchesRepo)
	if err != nil {
		return nil, err
	}

	chromiumVersion, _ := ReadChromiumVersion(patchesRepo)

	return &Context{
		Config:          cfg,
		State:           state,
		ChromiumDir:     chromiumDir,
		BrosDir:         brosDir,
		PatchesRepo:     patchesRepo,
		PatchesDir:      patchesDir,
		BaseCommit:      baseCommit,
		ChromiumVersion: chromiumVersion,
	}, nil
}

// LooksLikeChromium checks if a directory looks like a Chromium source tree.
func LooksLikeChromium(dir string) bool {
	markers := []string{"chrome", "base", ".git"}
	for _, m := range markers {
		if _, err := os.Stat(filepath.Join(dir, m)); err != nil {
			return false
		}
	}
	return true
}
