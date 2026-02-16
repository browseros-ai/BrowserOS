package dev

import (
	"bufio"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
)

type featureSelection struct {
	Name        string
	Description string
}

func runFeatureClassify(ctx *Context) error {
	if !isDirectory(ctx.PatchesDir()) {
		return fmt.Errorf("patches directory not found: %s", ctx.PatchesDir())
	}

	unclassified, err := getUnclassifiedFiles(ctx)
	if err != nil {
		return err
	}
	if len(unclassified) == 0 {
		fmt.Println("All patch files are already classified!")
		return nil
	}

	fmt.Printf("Found %d unclassified patch file(s)\n", len(unclassified))
	fmt.Println()

	classified, skipped, err := classifyFilesInteractive(ctx, unclassified)
	if err != nil {
		return err
	}

	fmt.Println()
	fmt.Println(strings.Repeat("=", 60))
	fmt.Printf("Classified %d file(s)\n", classified)
	if skipped > 0 {
		fmt.Printf("Skipped %d file(s)\n", skipped)
	}
	remaining := len(unclassified) - classified - skipped
	if remaining > 0 {
		fmt.Printf("Remaining: %d file(s)\n", remaining)
	}
	return nil
}

func classifyFilesInteractive(ctx *Context, unclassified []string) (int, int, error) {
	fmt.Println(strings.Repeat("=", 60))
	fmt.Println("Press Ctrl+C to stop at any time")
	fmt.Println()

	classifiedCount := 0
	skippedCount := 0
	reader := bufio.NewReader(os.Stdin)

	for i, filePath := range unclassified {
		fmt.Printf("\n[%d/%d] %s\n", i+1, len(unclassified), filePath)
		fmt.Println(strings.Repeat("-", 40))

		selection, err := promptFeatureSelectionForFile(ctx, reader, filePath)
		if err != nil {
			return classifiedCount, skippedCount, err
		}
		if selection == nil {
			fmt.Println("Skipped")
			skippedCount++
			continue
		}

		if _, err := addFilesToFeature(ctx, selection.Name, selection.Description, []string{filePath}); err != nil {
			return classifiedCount, skippedCount, err
		}
		classifiedCount++
	}

	return classifiedCount, skippedCount, nil
}

func promptFeatureSelectionForFile(ctx *Context, reader *bufio.Reader, filePath string) (*featureSelection, error) {
	features, _, err := loadFeaturesFile(ctx.FeaturesFile())
	if err != nil {
		return nil, err
	}

	if len(features.Features) == 0 {
		fmt.Println("No features defined yet. Create a new one:")
		return promptNewFeature(reader, "")
	}

	names := sortedFeatureNames(features.Features)
	for i, name := range names {
		feature := features.Features[name]
		if feature == nil {
			feature = &featureEntry{}
		}
		desc := feature.Description
		if strings.TrimSpace(desc) == "" {
			desc = name
		}
		fmt.Printf("  %d) %s (%d files)\n", i+1, desc, len(feature.Files))
	}

	newOption := len(names) + 1
	skipOption := len(names) + 2
	fmt.Printf("  %d) [Add new feature]\n", newOption)
	fmt.Printf("  %d) [Skip this file]\n", skipOption)

	for {
		choice, err := readLine(reader, fmt.Sprintf("Choice (1-%d): ", skipOption))
		if err != nil {
			if errors.Is(err, io.EOF) {
				return nil, nil
			}
			return nil, err
		}
		if strings.TrimSpace(choice) == "" {
			return nil, nil
		}

		choiceNum, err := strconv.Atoi(choice)
		if err != nil {
			fmt.Println("Enter a valid number")
			continue
		}
		if choiceNum < 1 || choiceNum > skipOption {
			fmt.Printf("Please enter 1-%d\n", skipOption)
			continue
		}

		switch choiceNum {
		case skipOption:
			return nil, nil
		case newOption:
			return promptNewFeature(reader, "")
		default:
			name := names[choiceNum-1]
			feature := features.Features[name]
			desc := ""
			if feature != nil {
				desc = feature.Description
			}
			return &featureSelection{Name: name, Description: desc}, nil
		}
	}
}

func promptNewFeature(reader *bufio.Reader, defaultDescription string) (*featureSelection, error) {
	fmt.Println()
	fmt.Println("Creating new feature:")
	fmt.Println(strings.Repeat("-", 40))
	fmt.Printf("  Valid prefixes: %s\n", strings.Join(validDescriptionPrefixes, ", "))
	fmt.Println()

	for {
		featureName, err := readLine(reader, "Feature name (kebab-case): ")
		if err != nil {
			if errors.Is(err, io.EOF) {
				fmt.Println("Cancelled")
				return nil, nil
			}
			return nil, err
		}
		if strings.TrimSpace(featureName) == "" {
			fmt.Println("Cancelled - no feature name provided")
			return nil, nil
		}

		sanitized := strings.ToLower(strings.ReplaceAll(strings.TrimSpace(featureName), " ", "-"))
		if err := validateFeatureName(sanitized); err != nil {
			fmt.Printf("Invalid name: %s\n", err)
			continue
		}

		for {
			descPrompt := "Description (e.g., feat: Add feature): "
			if strings.TrimSpace(defaultDescription) != "" {
				if err := validateDescription(defaultDescription); err == nil {
					descPrompt = fmt.Sprintf("Description [%s]: ", defaultDescription)
				} else {
					descPrompt = fmt.Sprintf("Description (e.g., feat: %s): ", defaultDescription)
				}
			}

			description, err := readLine(reader, descPrompt)
			if err != nil {
				if errors.Is(err, io.EOF) {
					fmt.Println("Cancelled")
					return nil, nil
				}
				return nil, err
			}

			description = strings.TrimSpace(description)
			if description == "" && strings.TrimSpace(defaultDescription) != "" {
				if err := validateDescription(defaultDescription); err == nil {
					description = defaultDescription
				} else {
					fmt.Printf("Default description needs prefix. Valid: %s\n", strings.Join(validDescriptionPrefixes, ", "))
					continue
				}
			}
			if description == "" {
				fmt.Printf("Description required. Must start with: %s\n", strings.Join(validDescriptionPrefixes, ", "))
				continue
			}
			if err := validateDescription(description); err != nil {
				fmt.Printf("Invalid description: %s\n", err)
				continue
			}

			return &featureSelection{
				Name:        sanitized,
				Description: description,
			}, nil
		}
	}
}

func getUnclassifiedFiles(ctx *Context) ([]string, error) {
	allPatchFiles, err := getAllPatchFiles(ctx)
	if err != nil {
		return nil, err
	}

	classified, err := getAllClassifiedFiles(ctx)
	if err != nil {
		return nil, err
	}

	unclassified := make([]string, 0)
	for _, file := range allPatchFiles {
		if _, ok := classified[file]; !ok {
			unclassified = append(unclassified, file)
		}
	}
	sort.Strings(unclassified)
	return unclassified, nil
}

func getAllPatchFiles(ctx *Context) ([]string, error) {
	root := ctx.PatchesDir()
	if !isDirectory(root) {
		return []string{}, nil
	}

	patchFiles := make([]string, 0)
	err := filepath.WalkDir(root, func(path string, d fs.DirEntry, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		if d.IsDir() {
			return nil
		}
		relPath, err := filepath.Rel(root, path)
		if err != nil {
			return err
		}
		patchFiles = append(patchFiles, filepath.ToSlash(relPath))
		return nil
	})
	if err != nil {
		return nil, fmt.Errorf("scanning patches directory: %w", err)
	}

	sort.Strings(patchFiles)
	return patchFiles, nil
}

func getAllClassifiedFiles(ctx *Context) (map[string]struct{}, error) {
	features, _, err := loadFeaturesFile(ctx.FeaturesFile())
	if err != nil {
		return nil, err
	}

	classified := make(map[string]struct{})
	for _, feature := range features.Features {
		if feature == nil {
			continue
		}
		for _, file := range normalizePaths(feature.Files) {
			classified[file] = struct{}{}
		}
	}
	return classified, nil
}

func readLine(reader *bufio.Reader, prompt string) (string, error) {
	fmt.Print(prompt)
	line, err := reader.ReadString('\n')
	if err != nil {
		if errors.Is(err, io.EOF) {
			line = strings.TrimSpace(line)
			if line == "" {
				return "", io.EOF
			}
			return line, nil
		}
		return "", err
	}
	return strings.TrimSpace(line), nil
}
