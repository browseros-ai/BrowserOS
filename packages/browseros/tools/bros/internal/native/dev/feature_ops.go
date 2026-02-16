package dev

import (
	"fmt"
	"sort"
	"strings"
)

func runFeatureList(ctx *Context) error {
	features, exists, err := loadFeaturesFile(ctx.FeaturesFile())
	if err != nil {
		return err
	}
	if !exists {
		fmt.Println("No features.yaml found")
		return nil
	}
	if len(features.Features) == 0 {
		fmt.Println("No features defined")
		return nil
	}

	fmt.Printf("Features (%d):\n", len(features.Features))
	fmt.Println(strings.Repeat("-", 60))

	for _, name := range sortedFeatureNames(features.Features) {
		config := features.Features[name]
		if config == nil {
			config = &featureEntry{}
		}
		fmt.Printf("  %s: %d files - %s\n", name, len(config.Files), config.Description)
	}
	return nil
}

func runFeatureShow(ctx *Context, featureName string) error {
	features, exists, err := loadFeaturesFile(ctx.FeaturesFile())
	if err != nil {
		return err
	}
	if !exists {
		fmt.Println("No features.yaml found")
		return nil
	}
	if len(features.Features) == 0 {
		fmt.Println("No features defined")
		return nil
	}

	feature, ok := features.Features[featureName]
	if !ok || feature == nil {
		fmt.Printf("Feature %q not found\n", featureName)
		fmt.Println("Available features:")
		for _, name := range sortedFeatureNames(features.Features) {
			fmt.Printf("  - %s\n", name)
		}
		return nil
	}

	commit := strings.TrimSpace(feature.Commit)
	if commit == "" {
		commit = "Unknown"
	}

	fmt.Printf("Feature: %s\n", featureName)
	fmt.Println(strings.Repeat("-", 60))
	fmt.Printf("Description: %s\n", feature.Description)
	fmt.Printf("Commit: %s\n", commit)
	fmt.Printf("Files (%d):\n", len(feature.Files))
	for _, filePath := range feature.Files {
		fmt.Printf("  - %s\n", filePath)
	}
	return nil
}

func runFeatureAddUpdate(ctx *Context, flags addUpdateFlags) error {
	if err := validateFeatureName(flags.Name); err != nil {
		return err
	}
	if err := validateDescription(flags.Description); err != nil {
		return err
	}

	changedFiles, err := commitChangedFiles(ctx.ChromiumSrc, flags.Commit)
	if err != nil {
		return err
	}
	if len(changedFiles) == 0 {
		return fmt.Errorf("No changed files found in commit %s", flags.Commit)
	}

	features, _, err := loadFeaturesFile(ctx.FeaturesFile())
	if err != nil {
		return err
	}

	existingFeature, exists := features.Features[flags.Name]
	if exists && existingFeature == nil {
		existingFeature = &featureEntry{}
	}

	if exists {
		existingFilesSet := toSet(existingFeature.Files)
		newFilesSet := toSet(changedFiles)
		addedFiles := setDifference(newFilesSet, existingFilesSet)
		alreadyPresent := setIntersection(newFilesSet, existingFilesSet)
		mergedFiles := setUnion(existingFilesSet, newFilesSet)

		fmt.Printf("Updating existing feature %q\n", flags.Name)
		fmt.Printf("  Current files: %d\n", len(existingFilesSet))
		fmt.Printf("  Files from commit: %d\n", len(newFilesSet))

		if len(addedFiles) > 0 {
			fmt.Printf("  Adding %d new file(s):\n", len(addedFiles))
			limit := min(10, len(addedFiles))
			for i := 0; i < limit; i++ {
				fmt.Printf("    + %s\n", addedFiles[i])
			}
			if len(addedFiles) > 10 {
				fmt.Printf("    ... and %d more\n", len(addedFiles)-10)
			}
		}
		if len(alreadyPresent) > 0 {
			fmt.Printf("  Skipping %d file(s) already in feature\n", len(alreadyPresent))
		}

		existingFeature.Files = mergedFiles
		existingFeature.Description = flags.Description
		features.Features[flags.Name] = existingFeature
	} else {
		fmt.Printf("Creating new feature %q\n", flags.Name)
		fmt.Printf("  Files from commit: %d\n", len(changedFiles))
		features.Features[flags.Name] = &featureEntry{
			Description: flags.Description,
			Files:       normalizePaths(changedFiles),
		}
	}

	if err := saveFeaturesFile(ctx.FeaturesFile(), features); err != nil {
		return err
	}

	totalFiles := len(features.Features[flags.Name].Files)
	if exists {
		fmt.Printf("Updated feature %q - now has %d files\n", flags.Name, totalFiles)
	} else {
		fmt.Printf("Created feature %q with %d files\n", flags.Name, totalFiles)
	}
	return nil
}

func addFilesToFeature(ctx *Context, featureName, description string, files []string) (int, error) {
	features, _, err := loadFeaturesFile(ctx.FeaturesFile())
	if err != nil {
		return 0, err
	}

	feature, exists := features.Features[featureName]
	if !exists || feature == nil {
		feature = &featureEntry{
			Description: strings.TrimSpace(description),
			Files:       []string{},
		}
		features.Features[featureName] = feature
	} else if strings.TrimSpace(feature.Description) == "" {
		feature.Description = strings.TrimSpace(description)
	}

	existing := toSet(feature.Files)
	newFiles := make([]string, 0, len(files))
	duplicates := make([]string, 0)
	for _, filePath := range normalizePaths(files) {
		if _, ok := existing[filePath]; ok {
			duplicates = append(duplicates, filePath)
			continue
		}
		existing[filePath] = struct{}{}
		newFiles = append(newFiles, filePath)
	}

	feature.Files = sortedKeys(existing)
	if err := saveFeaturesFile(ctx.FeaturesFile(), features); err != nil {
		return 0, err
	}

	if len(newFiles) > 0 {
		fmt.Printf("Added %d file(s) to feature %q\n", len(newFiles), featureName)
		limit := min(5, len(newFiles))
		for i := 0; i < limit; i++ {
			fmt.Printf("  + %s\n", newFiles[i])
		}
		if len(newFiles) > 5 {
			fmt.Printf("  ... and %d more\n", len(newFiles)-5)
		}
	}
	if len(duplicates) > 0 {
		fmt.Printf("Skipped %d duplicate file(s)\n", len(duplicates))
		limit := min(3, len(duplicates))
		for i := 0; i < limit; i++ {
			fmt.Printf("  ~ %s\n", duplicates[i])
		}
		if len(duplicates) > 3 {
			fmt.Printf("  ... and %d more\n", len(duplicates)-3)
		}
	}

	return len(newFiles), nil
}

func toSet(files []string) map[string]struct{} {
	set := make(map[string]struct{}, len(files))
	for _, file := range normalizePaths(files) {
		set[file] = struct{}{}
	}
	return set
}

func sortedKeys(set map[string]struct{}) []string {
	out := make([]string, 0, len(set))
	for k := range set {
		out = append(out, k)
	}
	sort.Strings(out)
	return out
}

func setDifference(a, b map[string]struct{}) []string {
	out := make([]string, 0)
	for k := range a {
		if _, ok := b[k]; !ok {
			out = append(out, k)
		}
	}
	sort.Strings(out)
	return out
}

func setIntersection(a, b map[string]struct{}) []string {
	out := make([]string, 0)
	for k := range a {
		if _, ok := b[k]; ok {
			out = append(out, k)
		}
	}
	sort.Strings(out)
	return out
}

func setUnion(a, b map[string]struct{}) []string {
	union := make(map[string]struct{}, len(a)+len(b))
	for k := range a {
		union[k] = struct{}{}
	}
	for k := range b {
		union[k] = struct{}{}
	}
	return sortedKeys(union)
}
