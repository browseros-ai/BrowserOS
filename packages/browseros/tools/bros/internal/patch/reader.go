package patch

import (
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
)

// ReadPatchSet reads all patches from the chromium_patches/ directory.
func ReadPatchSet(patchesDir string) (*PatchSet, error) {
	ps := NewPatchSet("")

	// Collect file paths
	var filePaths []string
	err := filepath.Walk(patchesDir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return nil
		}
		if !info.IsDir() {
			filePaths = append(filePaths, path)
		}
		return nil
	})
	if err != nil {
		return nil, err
	}

	// Sort deterministically so marker and patch-file merges are stable.
	sort.Strings(filePaths)

	for _, path := range filePaths {
		content, err := os.ReadFile(path)
		if err != nil {
			return nil, err
		}

		rel, err := filepath.Rel(patchesDir, path)
		if err != nil {
			return nil, err
		}

		fp := classifyPatchFile(rel, content)
		if existing, ok := ps.Patches[fp.Path]; ok {
			mergePatchFile(existing, fp)
		} else {
			ps.Patches[fp.Path] = fp
		}
	}

	return ps, nil
}

// ReadPatchFiles returns a map of chromium paths to true for all patches in the directory.
// Lighter than ReadPatchSet — only collects paths, not content.
func ReadPatchFiles(patchesDir string) (map[string]bool, error) {
	result := make(map[string]bool)

	err := filepath.Walk(patchesDir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return nil
		}
		if info.IsDir() {
			return nil
		}

		rel, err := filepath.Rel(patchesDir, path)
		if err != nil {
			return nil
		}

		chromPath := rel
		chromPath = strings.TrimSuffix(chromPath, ".deleted")
		chromPath = strings.TrimSuffix(chromPath, ".binary")
		chromPath = strings.TrimSuffix(chromPath, ".rename")

		result[chromPath] = true
		return nil
	})

	return result, err
}

func classifyPatchFile(rel string, content []byte) *FilePatch {
	fp := &FilePatch{
		Path:    rel,
		Content: content,
		Op:      OpModified,
	}

	switch {
	case strings.HasSuffix(rel, ".deleted"):
		fp.Path = strings.TrimSuffix(rel, ".deleted")
		fp.Op = OpDeleted
		fp.Content = nil
	case strings.HasSuffix(rel, ".binary"):
		fp.Path = strings.TrimSuffix(rel, ".binary")
		fp.Op = OpBinary
		fp.IsBinary = true
		fp.Content = nil
	case strings.HasSuffix(rel, ".rename"):
		fp.Path = strings.TrimSuffix(rel, ".rename")
		fp.Op = OpRenamed
		// Parse rename metadata from marker content.
		for _, line := range strings.Split(string(content), "\n") {
			if strings.HasPrefix(line, "rename_from: ") {
				fp.OldPath = strings.TrimPrefix(line, "rename_from: ")
			}
			if strings.HasPrefix(line, "similarity: ") {
				if sim, err := strconv.Atoi(strings.TrimPrefix(line, "similarity: ")); err == nil {
					fp.Similarity = sim
				}
			}
		}
		fp.Content = nil
	default:
		// Check if content looks like a diff with "new file mode"
		if strings.Contains(string(content), "new file mode") {
			fp.Op = OpAdded
		}
	}

	return fp
}

func mergePatchFile(dst, src *FilePatch) {
	if src == nil || dst == nil {
		return
	}

	// Keep real diff content whenever present.
	if src.Content != nil {
		dst.Content = src.Content
	}

	if src.OldPath != "" {
		dst.OldPath = src.OldPath
	}
	if src.Similarity != 0 {
		dst.Similarity = src.Similarity
	}
	if src.IsBinary {
		dst.IsBinary = true
	}

	// Preserve the strongest operation classification.
	if opPriority(src.Op) > opPriority(dst.Op) {
		dst.Op = src.Op
	}

	// Marker ops have no patch payload.
	if dst.Op == OpDeleted || dst.Op == OpBinary {
		dst.Content = nil
	}
}

func opPriority(op FileOp) int {
	switch op {
	case OpDeleted:
		return 5
	case OpBinary:
		return 4
	case OpRenamed:
		return 3
	case OpAdded:
		return 2
	default:
		return 1
	}
}
