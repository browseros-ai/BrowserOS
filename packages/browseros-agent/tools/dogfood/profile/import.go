package profile

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"

	"browseros-dogfood/internal/fspath"
)

type ImportConfig struct {
	SourceUserDataDir string
	SourceProfileDir  string
	DevUserDataDir    string
	DevProfileDir     string
}

var profileAllowlist = []string{
	"Extensions",
	"Extension State",
	"Extension Rules",
	"DNR Extension Rules",
	"Extension Scripts",
	"Local Extension Settings",
	"Sync Extension Settings",
	"Managed Extension Settings",
	"Login Data",
	"Login Data For Account",
	"Cookies",
	"Cookies-journal",
	"Bookmarks",
	"Preferences",
	"Web Data",
	"History",
}

var profileGlobAllowlist = []string{
	filepath.Join("IndexedDB", "chrome-extension_*"),
}

var warningOutput io.Writer = os.Stderr

func Import(cfg ImportConfig) error {
	if cfg.SourceUserDataDir == "" || cfg.SourceProfileDir == "" || cfg.DevUserDataDir == "" || cfg.DevProfileDir == "" {
		return fmt.Errorf("source and dev profile paths are required")
	}
	if fspath.IsSameOrChild(cfg.DevUserDataDir, cfg.SourceUserDataDir) {
		return fmt.Errorf("dev user-data dir must not equal or live inside source user-data dir")
	}
	sourceProfile := filepath.Join(cfg.SourceUserDataDir, cfg.SourceProfileDir)
	if info, err := os.Stat(sourceProfile); err != nil || !info.IsDir() {
		return fmt.Errorf("source profile not found: %s", sourceProfile)
	}
	if err := os.RemoveAll(cfg.DevUserDataDir); err != nil {
		return err
	}
	devProfile := filepath.Join(cfg.DevUserDataDir, cfg.DevProfileDir)
	if err := os.MkdirAll(devProfile, 0755); err != nil {
		return err
	}
	localStatePath := filepath.Join(cfg.DevUserDataDir, "Local State")
	if err := copyIfExists(filepath.Join(cfg.SourceUserDataDir, "Local State"), localStatePath); err != nil {
		return err
	}
	if err := patchLocalState(localStatePath, cfg.SourceProfileDir, cfg.DevProfileDir); err != nil {
		return err
	}
	for _, name := range profileAllowlist {
		src := filepath.Join(sourceProfile, name)
		dst := filepath.Join(devProfile, name)
		if err := copyIfExists(src, dst); err != nil {
			return err
		}
	}
	for _, pattern := range profileGlobAllowlist {
		if err := copyGlob(sourceProfile, devProfile, pattern); err != nil {
			return err
		}
	}
	if err := patchPreferences(filepath.Join(devProfile, "Preferences")); err != nil {
		return err
	}
	return CleanupSingletons(cfg.DevUserDataDir)
}

func CleanupSingletons(userDataDir string) error {
	entries, err := filepath.Glob(filepath.Join(userDataDir, "Singleton*"))
	if err != nil {
		return err
	}
	for _, entry := range entries {
		if err := os.RemoveAll(entry); err != nil {
			return err
		}
	}
	return nil
}

func HasSingletons(userDataDir string) (bool, error) {
	entries, err := filepath.Glob(filepath.Join(userDataDir, "Singleton*"))
	if err != nil {
		return false, err
	}
	return len(entries) > 0, nil
}

func copyIfExists(src string, dst string) error {
	info, err := os.Stat(src)
	if os.IsNotExist(err) {
		return nil
	}
	if err != nil {
		return err
	}
	if info.IsDir() {
		return copyDir(src, dst)
	}
	return copyFile(src, dst, info.Mode())
}

func copyGlob(srcRoot string, dstRoot string, pattern string) error {
	matches, err := filepath.Glob(filepath.Join(srcRoot, pattern))
	if err != nil {
		return err
	}
	for _, src := range matches {
		rel, err := filepath.Rel(srcRoot, src)
		if err != nil {
			return err
		}
		if err := copyIfExists(src, filepath.Join(dstRoot, rel)); err != nil {
			return err
		}
	}
	return nil
}

func copyDir(src string, dst string) error {
	return filepath.WalkDir(src, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		rel, err := filepath.Rel(src, path)
		if err != nil {
			return err
		}
		target := filepath.Join(dst, rel)
		info, err := d.Info()
		if err != nil {
			return err
		}
		if d.IsDir() {
			return os.MkdirAll(target, info.Mode())
		}
		return copyFile(path, target, info.Mode())
	})
}

func copyFile(src string, dst string, mode os.FileMode) error {
	if err := os.MkdirAll(filepath.Dir(dst), 0755); err != nil {
		return err
	}
	in, err := os.Open(src)
	if err != nil {
		return err
	}
	defer in.Close()
	out, err := os.OpenFile(dst, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, mode)
	if err != nil {
		return err
	}
	if _, err := io.Copy(out, in); err != nil {
		out.Close()
		return err
	}
	return out.Close()
}

func patchPreferences(path string) error {
	data, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		return nil
	}
	if err != nil {
		return err
	}
	var prefs map[string]any
	if err := json.Unmarshal(data, &prefs); err != nil {
		warnPatchSkipped(path, err)
		return nil
	}
	profile, ok := prefs["profile"].(map[string]any)
	if !ok {
		profile = map[string]any{}
		prefs["profile"] = profile
	}
	profile["exit_type"] = "Normal"
	profile["exited_cleanly"] = true
	out, err := json.Marshal(prefs)
	if err != nil {
		return err
	}
	return os.WriteFile(path, out, 0644)
}

func patchLocalState(path string, sourceProfileDir string, devProfileDir string) error {
	data, err := os.ReadFile(path)
	if os.IsNotExist(err) {
		return nil
	}
	if err != nil {
		return err
	}
	var state map[string]any
	if err := json.Unmarshal(data, &state); err != nil {
		warnPatchSkipped(path, err)
		return nil
	}
	profile := ensureObject(state, "profile")
	selected := selectedProfileInfo(profile, sourceProfileDir)
	profile["info_cache"] = map[string]any{devProfileDir: selected}
	profile["last_used"] = devProfileDir
	profile["last_active_profiles"] = []string{devProfileDir}
	profile["profiles_order"] = []string{devProfileDir}
	profile["show_picker_on_startup"] = false
	profile["picker_shown"] = true
	out, err := json.Marshal(state)
	if err != nil {
		return err
	}
	return os.WriteFile(path, out, 0644)
}

func warnPatchSkipped(path string, err error) {
	fmt.Fprintf(warningOutput, "warning: could not patch %s: invalid JSON: %v\n", path, err)
}

func ensureObject(parent map[string]any, key string) map[string]any {
	value, ok := parent[key].(map[string]any)
	if ok {
		return value
	}
	value = map[string]any{}
	parent[key] = value
	return value
}

func selectedProfileInfo(profile map[string]any, sourceProfileDir string) map[string]any {
	infoCache, ok := profile["info_cache"].(map[string]any)
	if !ok {
		return map[string]any{"name": sourceProfileDir}
	}
	selected, ok := infoCache[sourceProfileDir].(map[string]any)
	if !ok {
		return map[string]any{"name": sourceProfileDir}
	}
	return selected
}
