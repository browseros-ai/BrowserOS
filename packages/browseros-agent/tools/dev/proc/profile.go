package proc

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// DefaultDevUserDataDir returns the stable browser profile for this checkout.
func DefaultDevUserDataDir(root string) (string, error) {
	absRoot, err := filepath.Abs(root)
	if err != nil {
		return "", err
	}
	sum := sha256.Sum256([]byte(absRoot))
	key := hex.EncodeToString(sum[:])[:8]
	return filepath.Join(os.TempDir(), fmt.Sprintf("browseros-dev-%s-%s", worktreeLabel(absRoot), key)), nil
}

func worktreeLabel(root string) string {
	worktree := root
	if filepath.Base(root) == "browseros-agent" && filepath.Base(filepath.Dir(root)) == "packages" {
		worktree = filepath.Dir(filepath.Dir(root))
	}
	label := sanitizeProfileLabel(filepath.Base(worktree))
	if label == "" {
		return "repo"
	}
	return label
}

func sanitizeProfileLabel(value string) string {
	var builder strings.Builder
	lastDash := false
	for _, r := range strings.ToLower(value) {
		if (r >= 'a' && r <= 'z') || (r >= '0' && r <= '9') || r == '_' || r == '.' {
			builder.WriteRune(r)
			lastDash = false
			continue
		}
		if !lastDash {
			builder.WriteByte('-')
			lastDash = true
		}
	}
	return strings.Trim(builder.String(), "-")
}
