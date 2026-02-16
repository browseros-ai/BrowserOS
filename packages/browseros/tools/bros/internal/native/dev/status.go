package dev

import (
	"fmt"
	"io/fs"
	"path/filepath"
	"strings"
)

func runStatus(ctx *Context) error {
	fmt.Println("Dev CLI Status")
	fmt.Println(strings.Repeat("-", 40))
	fmt.Printf("Chromium source: %s\n", ctx.ChromiumSrc)

	patchesDir := ctx.PatchesDir()
	if !isDirectory(patchesDir) {
		fmt.Println("No patches directory found")
	} else {
		patchCount, err := countPatchFiles(patchesDir)
		if err != nil {
			return err
		}
		fmt.Printf("Individual patches: %d\n", patchCount)
	}

	features, exists, err := loadFeaturesFile(ctx.FeaturesFile())
	if err != nil {
		return err
	}
	if !exists {
		fmt.Println("No features.yaml found")
	} else {
		fmt.Printf("Features defined: %d\n", len(features.Features))
	}

	return nil
}

func countPatchFiles(root string) (int, error) {
	count := 0
	err := filepath.WalkDir(root, func(path string, d fs.DirEntry, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		if d.IsDir() {
			return nil
		}
		if strings.HasSuffix(path, ".patch") {
			count++
		}
		return nil
	})
	if err != nil {
		return 0, fmt.Errorf("counting patches in %s: %w", root, err)
	}
	return count, nil
}
