package patch

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"runtime"

	"golang.org/x/sync/errgroup"
)

// WritePatchSet writes patches to the chromium_patches/ directory.
func WritePatchSet(patchesDir string, ps *PatchSet, dryRun bool) error {
	g, _ := errgroup.WithContext(context.Background())
	g.SetLimit(runtime.NumCPU())

	for _, fp := range ps.Patches {
		fp := fp
		g.Go(func() error {
			return writeSinglePatch(patchesDir, fp, dryRun)
		})
	}

	return g.Wait()
}

func writeSinglePatch(patchesDir string, fp *FilePatch, dryRun bool) error {
	if dryRun {
		return nil
	}

	switch fp.Op {
	case OpDeleted:
		if err := cleanupArtifacts(patchesDir, fp.Path, fp.Path+".deleted"); err != nil {
			return err
		}
		return writeDeletedMarker(patchesDir, fp)
	case OpBinary:
		if err := cleanupArtifacts(patchesDir, fp.Path, fp.Path+".binary"); err != nil {
			return err
		}
		return writeBinaryMarker(patchesDir, fp)
	case OpRenamed:
		if err := cleanupArtifacts(patchesDir, fp.Path, fp.Path, fp.Path+".rename"); err != nil {
			return err
		}
		if err := writePatchFile(patchesDir, fp); err != nil {
			return err
		}
		return writeRenameMarker(patchesDir, fp)
	default:
		if err := cleanupArtifacts(patchesDir, fp.Path, fp.Path); err != nil {
			return err
		}
		return writePatchFile(patchesDir, fp)
	}
}

func writePatchFile(patchesDir string, fp *FilePatch) error {
	dest := filepath.Join(patchesDir, fp.Path)
	if err := os.MkdirAll(filepath.Dir(dest), 0o755); err != nil {
		return fmt.Errorf("creating directory: %w", err)
	}
	return os.WriteFile(dest, fp.Content, 0o644)
}

func writeDeletedMarker(patchesDir string, fp *FilePatch) error {
	dest := filepath.Join(patchesDir, fp.Path+".deleted")
	if err := os.MkdirAll(filepath.Dir(dest), 0o755); err != nil {
		return fmt.Errorf("creating directory: %w", err)
	}
	content := fmt.Sprintf("deleted: %s\n", fp.Path)
	return os.WriteFile(dest, []byte(content), 0o644)
}

func writeBinaryMarker(patchesDir string, fp *FilePatch) error {
	dest := filepath.Join(patchesDir, fp.Path+".binary")
	if err := os.MkdirAll(filepath.Dir(dest), 0o755); err != nil {
		return fmt.Errorf("creating directory: %w", err)
	}
	content := fmt.Sprintf("binary: %s\n", fp.Path)
	return os.WriteFile(dest, []byte(content), 0o644)
}

func writeRenameMarker(patchesDir string, fp *FilePatch) error {
	dest := filepath.Join(patchesDir, fp.Path+".rename")
	if err := os.MkdirAll(filepath.Dir(dest), 0o755); err != nil {
		return fmt.Errorf("creating directory: %w", err)
	}
	content := fmt.Sprintf("rename_from: %s\nsimilarity: %d\n", fp.OldPath, fp.Similarity)
	return os.WriteFile(dest, []byte(content), 0o644)
}

func cleanupArtifacts(patchesDir, path string, keep ...string) error {
	keepSet := make(map[string]bool, len(keep))
	for _, k := range keep {
		keepSet[k] = true
	}

	candidates := []string{
		path,
		path + ".deleted",
		path + ".binary",
		path + ".rename",
	}

	for _, rel := range candidates {
		if keepSet[rel] {
			continue
		}
		full := filepath.Join(patchesDir, rel)
		if err := os.Remove(full); err != nil && !os.IsNotExist(err) {
			return fmt.Errorf("removing stale artifact %s: %w", rel, err)
		}
	}

	return nil
}
