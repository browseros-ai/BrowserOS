package workspace

import (
	"errors"
	"fmt"
	"path/filepath"
	"slices"
	"strings"
)

// Detect finds the registered Chromium checkout that contains cwd.
func Detect(reg *Registry, cwd string) (Entry, error) {
	return DetectForCommand(reg, cwd, "browseros-patch diff")
}

// DetectForCommand finds the checkout for cwd and includes a command-specific
// named-checkout example when cwd is not registered.
func DetectForCommand(reg *Registry, cwd string, commandPath string) (Entry, error) {
	if len(reg.Workspaces) == 0 {
		return Entry{}, fmt.Errorf(`no Chromium checkouts registered; run "browseros-patch add <name> <path>"`)
	}
	abs, err := filepath.Abs(cwd)
	if err != nil {
		return Entry{}, err
	}
	clean := filepath.Clean(abs)
	realClean := canonicalPath(clean)
	var best Entry
	bestLen := -1
	for _, ws := range reg.Workspaces {
		base := filepath.Clean(ws.Path)
		realBase := canonicalPath(base)
		if containsPath(clean, base) || containsPath(realClean, realBase) {
			if len(realBase) > bestLen {
				best = ws
				bestLen = len(realBase)
			}
		}
	}
	if bestLen == -1 {
		return Entry{}, errors.New(detectErrorMessage(reg, clean, realClean, commandPath))
	}
	return best, nil
}

// Resolve resolves a checkout from --src, an explicit name, or cwd detection.
func Resolve(reg *Registry, name string, cwd string, src string) (Entry, error) {
	return ResolveForCommand(reg, name, cwd, src, "browseros-patch diff")
}

// ResolveForCommand resolves a checkout and tailors cwd detection errors for a
// specific command such as "browseros-patch diff".
func ResolveForCommand(reg *Registry, name string, cwd string, src string, commandPath string) (Entry, error) {
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
	return DetectForCommand(reg, cwd, commandPath)
}

func detectErrorMessage(reg *Registry, cleanCWD string, resolvedCWD string, commandPath string) string {
	var builder strings.Builder
	builder.WriteString("not inside a registered Chromium checkout\n")
	builder.WriteString("cwd: " + cleanCWD)
	if resolvedCWD != cleanCWD {
		builder.WriteString("\nresolved cwd: " + resolvedCWD)
	}
	builder.WriteString("\nregistered checkouts:")
	sorted := append([]Entry(nil), reg.Workspaces...)
	slices.SortFunc(sorted, func(a, b Entry) int {
		return strings.Compare(a.Name, b.Name)
	})
	for _, ws := range sorted {
		builder.WriteString(fmt.Sprintf("\n  %s  %s", ws.Name, ws.Path))
	}
	builder.WriteString("\ntry: " + namedCheckoutExample(sorted, commandPath))
	return builder.String()
}

func namedCheckoutExample(workspaces []Entry, commandPath string) string {
	commandPath = strings.TrimSpace(commandPath)
	if commandPath == "" {
		commandPath = "browseros-patch diff"
	}
	return commandPath + " " + workspaces[0].Name
}

func canonicalPath(path string) string {
	realPath, err := filepath.EvalSymlinks(path)
	if err != nil {
		return filepath.Clean(path)
	}
	return filepath.Clean(realPath)
}

func containsPath(path string, base string) bool {
	return path == base || strings.HasPrefix(path, base+string(filepath.Separator))
}
