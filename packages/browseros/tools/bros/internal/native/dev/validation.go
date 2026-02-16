package dev

import (
	"fmt"
	"regexp"
	"strings"
)

var validDescriptionPrefixes = []string{"feat:", "fix:", "build:", "chore:", "series:"}

var featureNamePattern = regexp.MustCompile(`^[a-z0-9][a-z0-9_-]*$`)

func validateDescription(description string) error {
	description = strings.TrimSpace(description)
	if description == "" {
		return fmt.Errorf("description cannot be empty")
	}

	for _, prefix := range validDescriptionPrefixes {
		if strings.HasPrefix(description, prefix) {
			return nil
		}
	}

	return fmt.Errorf("description must start with one of: %s", strings.Join(validDescriptionPrefixes, ", "))
}

func validateFeatureName(name string) error {
	if strings.TrimSpace(name) == "" {
		return fmt.Errorf("feature name cannot be empty")
	}

	if strings.Contains(name, " ") {
		return fmt.Errorf("feature name cannot contain spaces (use hyphens instead)")
	}

	if strings.Contains(name, ":") {
		return fmt.Errorf("feature name cannot contain ':' (did you pass a description as the name?)")
	}

	if name != strings.ToLower(name) {
		return fmt.Errorf("feature name must be lowercase (got %q, use %q)", name, strings.ToLower(name))
	}

	if !featureNamePattern.MatchString(name) {
		return fmt.Errorf("feature name must start with a letter/number and contain only lowercase letters, numbers, hyphens, and underscores")
	}

	return nil
}
