package dev

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

func runAnnotate(ctx *Context, featureFilter string) error {
	fmt.Println("Annotate Features")
	fmt.Println(strings.Repeat("=", 60))
	fmt.Printf("Chromium source: %s\n", ctx.ChromiumSrc)
	fmt.Printf("Features file: %s\n", ctx.FeaturesFile())

	commitsCreated, featuresSkipped, err := annotateFeatures(ctx, featureFilter)
	if err != nil {
		return err
	}

	fmt.Println()
	fmt.Println(strings.Repeat("=", 60))
	if commitsCreated > 0 {
		fmt.Printf("Created %d commit(s)\n", commitsCreated)
	} else {
		fmt.Println("No commits created (no modified files found)")
	}
	if featuresSkipped > 0 {
		fmt.Printf("Skipped %d feature(s) with no changes\n", featuresSkipped)
	}
	fmt.Println(strings.Repeat("=", 60))
	return nil
}

func annotateFeatures(ctx *Context, featureFilter string) (int, int, error) {
	features, exists, err := loadFeaturesFile(ctx.FeaturesFile())
	if err != nil {
		return 0, 0, err
	}
	if !exists {
		fmt.Printf("Features file not found: %s\n", ctx.FeaturesFile())
		return 0, 0, nil
	}
	if len(features.Features) == 0 {
		fmt.Println("No features found in features.yaml")
		return 0, 0, nil
	}

	selected := make(map[string]*featureEntry)
	if strings.TrimSpace(featureFilter) != "" {
		feature, ok := features.Features[featureFilter]
		if !ok {
			fmt.Printf("Feature %q not found in features.yaml\n", featureFilter)
			return 0, 0, nil
		}
		selected[featureFilter] = feature
	} else {
		for name, feature := range features.Features {
			selected[name] = feature
		}
	}

	fmt.Printf("Processing %d feature(s)\n", len(selected))
	fmt.Println(strings.Repeat("=", 60))

	commitsCreated := 0
	featuresSkipped := 0

	for _, featureName := range sortedFeatureNames(selected) {
		feature := selected[featureName]
		if feature == nil {
			feature = &featureEntry{}
		}
		description := strings.TrimSpace(feature.Description)
		if description == "" {
			description = featureName
		}
		files := normalizePaths(feature.Files)

		fmt.Println()
		fmt.Printf("%s\n", featureName)
		fmt.Printf("   %s\n", description)

		if len(files) == 0 {
			fmt.Println("   No files specified, skipping")
			featuresSkipped++
			continue
		}

		modifiedFiles, err := modifiedFilesForFeature(ctx.ChromiumSrc, files)
		if err != nil {
			fmt.Printf("   Failed to inspect files: %v\n", err)
			featuresSkipped++
			continue
		}
		if len(modifiedFiles) == 0 {
			fmt.Printf("   No modified files (%d files checked)\n", len(files))
			featuresSkipped++
			continue
		}

		fmt.Printf("   Found %d modified file(s)\n", len(modifiedFiles))
		committed, err := gitAddAndCommit(ctx.ChromiumSrc, modifiedFiles, description)
		if err != nil {
			fmt.Printf("   Failed to commit: %v\n", err)
			featuresSkipped++
			continue
		}
		if !committed {
			fmt.Println("   No changes staged, skipping commit")
			featuresSkipped++
			continue
		}

		fmt.Printf("   Committed %d file(s)\n", len(modifiedFiles))
		commitsCreated++
	}

	return commitsCreated, featuresSkipped, nil
}

func modifiedFilesForFeature(chromiumSrc string, files []string) ([]string, error) {
	modified := make([]string, 0)
	seen := map[string]struct{}{}

	for _, filePath := range files {
		if _, ok := seen[filePath]; ok {
			continue
		}
		seen[filePath] = struct{}{}

		fullPath := filepath.Join(chromiumSrc, filepath.FromSlash(filePath))
		if _, err := os.Stat(fullPath); err != nil {
			continue
		}

		result, err := runGit(chromiumSrc, "status", "--porcelain", "--", filePath)
		if err != nil {
			return nil, err
		}
		if result.ExitCode == 0 && strings.TrimSpace(result.Stdout) != "" {
			modified = append(modified, filePath)
		}
	}

	return modified, nil
}

func gitAddAndCommit(chromiumSrc string, files []string, commitMessage string) (bool, error) {
	for _, filePath := range files {
		result, err := runGit(chromiumSrc, "add", "--", filePath)
		if err != nil {
			return false, err
		}
		if result.ExitCode != 0 {
			return false, fmt.Errorf("failed to add file %q: %s", filePath, strings.TrimSpace(result.Stderr))
		}
	}

	commitResult, err := runGit(chromiumSrc, "commit", "-m", commitMessage)
	if err != nil {
		return false, err
	}
	if commitResult.ExitCode != 0 {
		combined := strings.ToLower(strings.TrimSpace(commitResult.Stderr + "\n" + commitResult.Stdout))
		if strings.Contains(combined, "nothing to commit") || strings.Contains(combined, "nothing added to commit") {
			return false, nil
		}
		msg := strings.TrimSpace(commitResult.Stderr)
		if msg == "" {
			msg = "git commit failed"
		}
		return false, errors.New(msg)
	}

	return true, nil
}
