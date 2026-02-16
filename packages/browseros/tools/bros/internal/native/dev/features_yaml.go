package dev

import (
	"bytes"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"gopkg.in/yaml.v3"
)

type featureEntry struct {
	Description string
	Files       []string
	Commit      string
}

type featuresFile struct {
	Version  string
	Features map[string]*featureEntry
}

func loadFeaturesFile(path string) (*featuresFile, bool, error) {
	if !isRegularFile(path) {
		return &featuresFile{
			Version:  "1.0",
			Features: map[string]*featureEntry{},
		}, false, nil
	}

	data, err := os.ReadFile(path)
	if err != nil {
		return nil, true, fmt.Errorf("reading features file: %w", err)
	}

	var raw struct {
		Version  string `yaml:"version"`
		Features map[string]struct {
			Description string   `yaml:"description"`
			Files       []string `yaml:"files"`
			Commit      string   `yaml:"commit"`
		} `yaml:"features"`
	}

	if err := yaml.Unmarshal(data, &raw); err != nil {
		return nil, true, fmt.Errorf("parsing features file: %w", err)
	}

	out := &featuresFile{
		Version:  strings.TrimSpace(raw.Version),
		Features: map[string]*featureEntry{},
	}
	if out.Version == "" {
		out.Version = "1.0"
	}

	for name, feature := range raw.Features {
		out.Features[name] = &featureEntry{
			Description: strings.TrimSpace(feature.Description),
			Files:       normalizePaths(feature.Files),
			Commit:      strings.TrimSpace(feature.Commit),
		}
	}

	return out, true, nil
}

func saveFeaturesFile(path string, doc *featuresFile) error {
	if doc == nil {
		return fmt.Errorf("features document is nil")
	}
	if strings.TrimSpace(doc.Version) == "" {
		doc.Version = "1.0"
	}
	if doc.Features == nil {
		doc.Features = map[string]*featureEntry{}
	}

	root := &yaml.Node{Kind: yaml.MappingNode}
	appendMappingScalar(root, "version", doc.Version)

	featuresNode := &yaml.Node{Kind: yaml.MappingNode}
	names := sortedFeatureNames(doc.Features)
	for _, name := range names {
		feature := doc.Features[name]
		if feature == nil {
			feature = &featureEntry{}
		}

		featureNode := &yaml.Node{Kind: yaml.MappingNode}
		if strings.TrimSpace(feature.Description) != "" {
			appendMappingScalar(featureNode, "description", strings.TrimSpace(feature.Description))
		}
		if strings.TrimSpace(feature.Commit) != "" {
			appendMappingScalar(featureNode, "commit", strings.TrimSpace(feature.Commit))
		}

		files := normalizePaths(feature.Files)
		filesNode := &yaml.Node{Kind: yaml.SequenceNode}
		for _, f := range files {
			filesNode.Content = append(filesNode.Content, &yaml.Node{
				Kind:  yaml.ScalarNode,
				Tag:   "!!str",
				Value: f,
			})
		}
		appendMappingNode(featureNode, "files", filesNode)
		appendMappingNode(featuresNode, name, featureNode)
	}
	appendMappingNode(root, "features", featuresNode)

	var buf bytes.Buffer
	enc := yaml.NewEncoder(&buf)
	enc.SetIndent(2)
	if err := enc.Encode(root); err != nil {
		return fmt.Errorf("encoding features file: %w", err)
	}
	_ = enc.Close()

	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		return fmt.Errorf("creating features directory: %w", err)
	}
	if err := os.WriteFile(path, buf.Bytes(), 0o644); err != nil {
		return fmt.Errorf("writing features file: %w", err)
	}

	return nil
}

func appendMappingScalar(node *yaml.Node, key, value string) {
	appendMappingNode(node, key, &yaml.Node{
		Kind:  yaml.ScalarNode,
		Tag:   "!!str",
		Value: value,
	})
}

func appendMappingNode(node *yaml.Node, key string, value *yaml.Node) {
	node.Content = append(node.Content,
		&yaml.Node{Kind: yaml.ScalarNode, Tag: "!!str", Value: key},
		value,
	)
}

func sortedFeatureNames(features map[string]*featureEntry) []string {
	names := make([]string, 0, len(features))
	for name := range features {
		names = append(names, name)
	}
	sort.Strings(names)
	return names
}

func normalizePaths(paths []string) []string {
	out := make([]string, 0, len(paths))
	seen := make(map[string]struct{}, len(paths))
	for _, p := range paths {
		normalized := filepath.ToSlash(strings.TrimSpace(p))
		if normalized == "" {
			continue
		}
		if _, ok := seen[normalized]; ok {
			continue
		}
		seen[normalized] = struct{}{}
		out = append(out, normalized)
	}
	sort.Strings(out)
	return out
}
