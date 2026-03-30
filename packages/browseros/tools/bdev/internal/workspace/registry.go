package workspace

import (
	"fmt"
	"os"
	"path/filepath"
	"slices"
	"time"

	"gopkg.in/yaml.v3"
)

type Entry struct {
	Name    string    `yaml:"name" json:"name"`
	Path    string    `yaml:"path" json:"path"`
	AddedAt time.Time `yaml:"added_at,omitempty" json:"added_at,omitempty"`
}

type Registry struct {
	Version    int     `yaml:"version" json:"version"`
	Workspaces []Entry `yaml:"workspaces" json:"workspaces"`
}

func LoadRegistry() (*Registry, error) {
	data, err := os.ReadFile(RegistryPath())
	if err != nil {
		if os.IsNotExist(err) {
			return &Registry{Version: 1}, nil
		}
		return nil, err
	}
	var reg Registry
	if err := yaml.Unmarshal(data, &reg); err != nil {
		return nil, fmt.Errorf("parse registry: %w", err)
	}
	if reg.Version == 0 {
		reg.Version = 1
	}
	return &reg, nil
}

func SaveRegistry(reg *Registry) error {
	if reg.Version == 0 {
		reg.Version = 1
	}
	if err := os.MkdirAll(ConfigDir(), 0o755); err != nil {
		return err
	}
	body, err := yaml.Marshal(reg)
	if err != nil {
		return err
	}
	header := "# bdev workspaces\n\n"
	return os.WriteFile(RegistryPath(), append([]byte(header), body...), 0o644)
}

func NormalizeWorkspacePath(raw string) (string, error) {
	abs, err := filepath.Abs(raw)
	if err != nil {
		return "", err
	}
	clean := filepath.Clean(abs)
	info, err := os.Stat(clean)
	if err != nil {
		return "", err
	}
	if !info.IsDir() {
		return "", fmt.Errorf("workspace path is not a directory: %s", clean)
	}
	if _, err := os.Stat(filepath.Join(clean, ".git")); err != nil {
		return "", fmt.Errorf("workspace is not a git checkout: %s", clean)
	}
	return clean, nil
}

func (r *Registry) Get(name string) (Entry, error) {
	for _, ws := range r.Workspaces {
		if ws.Name == name {
			return ws, nil
		}
	}
	return Entry{}, fmt.Errorf("workspace %q not found", name)
}

func (r *Registry) Add(name string, path string) (Entry, error) {
	normalized, err := NormalizeWorkspacePath(path)
	if err != nil {
		return Entry{}, err
	}
	for _, ws := range r.Workspaces {
		switch {
		case ws.Name == name:
			return Entry{}, fmt.Errorf("workspace %q already exists", name)
		case ws.Path == normalized:
			return Entry{}, fmt.Errorf("workspace path already registered as %q", ws.Name)
		}
	}
	entry := Entry{Name: name, Path: normalized, AddedAt: time.Now().UTC()}
	r.Workspaces = append(r.Workspaces, entry)
	slices.SortFunc(r.Workspaces, func(a, b Entry) int {
		if a.Name < b.Name {
			return -1
		}
		if a.Name > b.Name {
			return 1
		}
		return 0
	})
	return entry, nil
}

func (r *Registry) Remove(name string) (Entry, error) {
	for idx, ws := range r.Workspaces {
		if ws.Name != name {
			continue
		}
		r.Workspaces = append(r.Workspaces[:idx], r.Workspaces[idx+1:]...)
		return ws, nil
	}
	return Entry{}, fmt.Errorf("workspace %q not found", name)
}
