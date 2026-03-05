package cmd

import (
	"fmt"
	"strings"

	"bdev/internal/git"
)

func resolveRemoteAndFiles(repoDir string, args []string, explicitRemote string) (string, []string, error) {
	remote := strings.TrimSpace(explicitRemote)
	if remote != "" {
		hasRemote, err := git.HasRemote(repoDir, remote)
		if err != nil {
			return "", nil, fmt.Errorf("resolving remote %q: %w", remote, err)
		}
		if !hasRemote {
			return "", nil, fmt.Errorf("remote %q not found in patches repo", remote)
		}
		return remote, args, nil
	}

	if len(args) == 0 {
		return "", nil, nil
	}

	hasRemote, err := git.HasRemote(repoDir, args[0])
	if err != nil {
		return "", nil, fmt.Errorf("resolving remote %q: %w", args[0], err)
	}
	if hasRemote {
		return args[0], args[1:], nil
	}

	return "", args, nil
}

func shortRev(rev string) string {
	rev = strings.TrimSpace(rev)
	if len(rev) <= 12 {
		return rev
	}
	return rev[:12]
}
