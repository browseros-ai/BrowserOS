package workspace

import (
	"fmt"
	"path/filepath"
	"strings"
)

func Detect(reg *Registry, cwd string) (Entry, error) {
	if len(reg.Workspaces) == 0 {
		return Entry{}, fmt.Errorf("no workspaces registered yet")
	}
	abs, err := filepath.Abs(cwd)
	if err != nil {
		return Entry{}, err
	}
	clean := filepath.Clean(abs)
	var best Entry
	bestLen := -1
	for _, ws := range reg.Workspaces {
		base := filepath.Clean(ws.Path)
		if clean == base || strings.HasPrefix(clean, base+string(filepath.Separator)) {
			if len(base) > bestLen {
				best = ws
				bestLen = len(base)
			}
		}
	}
	if bestLen == -1 {
		return Entry{}, fmt.Errorf(
			`not inside a registered workspace; run "bdev list" to inspect workspaces or pass one by name`,
		)
	}
	return best, nil
}

func Resolve(reg *Registry, name string, cwd string, src string) (Entry, error) {
	if src != "" {
		path, err := NormalizeWorkspacePath(src)
		if err != nil {
			return Entry{}, err
		}
		return Entry{Name: filepath.Base(path), Path: path}, nil
	}
	if name != "" {
		return reg.Get(name)
	}
	return Detect(reg, cwd)
}
