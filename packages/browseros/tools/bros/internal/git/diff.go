package git

import (
	"fmt"
	"strings"

	"bros/internal/patch"
)

// DiffNameStatus returns file paths mapped to their operation.
// Diffs BASE against the working tree (not HEAD) so uncommitted patch
// applications are visible.
func DiffNameStatus(dir, base string) (map[string]patch.FileOp, error) {
	out, err := Run(dir, "diff", "--name-status", "-M", base)
	if err != nil {
		return nil, fmt.Errorf("diff --name-status %s: %w", base, err)
	}

	result := make(map[string]patch.FileOp)
	for _, line := range strings.Split(out, "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}

		fields := strings.Fields(line)
		if len(fields) < 2 {
			continue
		}

		status := fields[0]
		path := fields[len(fields)-1]

		switch {
		case status == "M":
			result[path] = patch.OpModified
		case status == "A":
			result[path] = patch.OpAdded
		case status == "D":
			result[path] = patch.OpDeleted
		case strings.HasPrefix(status, "R"):
			result[path] = patch.OpRenamed
		default:
			result[path] = patch.OpModified
		}
	}

	return result, nil
}

// DiffFull returns the complete unified diff of BASE vs working tree.
func DiffFull(dir, base string) ([]byte, error) {
	out, err := RunBytes(dir, "diff", "-M", "--full-index", base)
	if err != nil {
		return nil, fmt.Errorf("diff %s: %w", base, err)
	}
	return out, nil
}

// DiffFile returns the working-tree diff for a single file.
func DiffFile(dir, base, file string) ([]byte, error) {
	out, err := RunBytes(dir, "diff", "-M", "--full-index", base, "--", file)
	if err != nil {
		return nil, fmt.Errorf("diff %s -- %s: %w", base, file, err)
	}
	return out, nil
}

// DiffFiles returns working-tree diffs for multiple files in one call.
func DiffFiles(dir, base string, files []string) ([]byte, error) {
	args := []string{"diff", "-M", "--full-index", base, "--"}
	args = append(args, files...)
	out, err := RunBytes(dir, args...)
	if err != nil {
		return nil, fmt.Errorf("diff %s -- [%d files]: %w", base, len(files), err)
	}
	return out, nil
}
