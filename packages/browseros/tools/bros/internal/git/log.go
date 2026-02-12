package git

import (
	"fmt"
	"strings"
)

// ChangedFilesSince returns files changed between two revs in a subdirectory.
func ChangedFilesSince(dir, fromRev, toRev, subdir string) ([]string, error) {
	args := []string{"diff", "--name-only", fromRev + ".." + toRev}
	if subdir != "" {
		args = append(args, "--", subdir)
	}

	out, err := Run(dir, args...)
	if err != nil {
		return nil, fmt.Errorf("diff --name-only %s..%s: %w", fromRev, toRev, err)
	}

	if out == "" {
		return nil, nil
	}

	var files []string
	for _, line := range strings.Split(out, "\n") {
		line = strings.TrimSpace(line)
		if line != "" {
			files = append(files, line)
		}
	}
	return files, nil
}
