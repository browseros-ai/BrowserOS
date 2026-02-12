package patch

import (
	"context"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"

	"golang.org/x/sync/errgroup"
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

	g, _ := errgroup.WithContext(context.Background())
	g.SetLimit(runtime.NumCPU())

	var mu sync.Mutex

	for _, path := range filePaths {
		path := path
		g.Go(func() error {
			content, err := os.ReadFile(path)
			if err != nil {
				return err
			}

			rel, err := filepath.Rel(patchesDir, path)
			if err != nil {
				return err
			}

			fp := classifyPatchFile(rel, content)
			mu.Lock()
			ps.Patches[fp.Path] = fp
			mu.Unlock()
			return nil
		})
	}

	if err := g.Wait(); err != nil {
		return nil, err
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
		// Parse rename_from from content
		for _, line := range strings.Split(string(content), "\n") {
			if strings.HasPrefix(line, "rename_from: ") {
				fp.OldPath = strings.TrimPrefix(line, "rename_from: ")
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
