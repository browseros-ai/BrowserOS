package patch

import (
	"fmt"
	"regexp"
	"strconv"
	"strings"
)

var diffHeader = regexp.MustCompile(`^diff --git a/(.*) b/(.*)$`)

func ParseDiffOutput(diff string) (PatchSet, error) {
	patches := PatchSet{}
	lines := strings.SplitAfter(diff, "\n")
	var current *FilePatch
	var buffer strings.Builder

	flush := func() error {
		if current == nil {
			return nil
		}
		current.Content = []byte(buffer.String())
		if current.Path == "" {
			return fmt.Errorf("diff patch missing target path")
		}
		patches[current.Path] = *current
		current = nil
		buffer.Reset()
		return nil
	}

	for _, line := range lines {
		if strings.HasPrefix(line, "diff --git ") {
			if err := flush(); err != nil {
				return nil, err
			}
			match := diffHeader.FindStringSubmatch(strings.TrimRight(line, "\n"))
			if len(match) != 3 {
				return nil, fmt.Errorf("invalid diff header: %s", strings.TrimSpace(line))
			}
			current = &FilePatch{
				Path: NormalizeChromiumPath(match[2]),
				Op:   OpModify,
			}
		}
		if current == nil {
			continue
		}
		buffer.WriteString(line)
		trimmed := strings.TrimRight(line, "\n")
		switch {
		case strings.HasPrefix(trimmed, "new file mode "):
			current.Op = OpAdd
		case strings.HasPrefix(trimmed, "deleted file mode "):
			current.Op = OpDelete
		case strings.HasPrefix(trimmed, "rename from "):
			current.Op = OpRename
			current.OldPath = NormalizeChromiumPath(strings.TrimPrefix(trimmed, "rename from "))
		case strings.HasPrefix(trimmed, "copy from "):
			current.Op = OpCopy
			current.OldPath = NormalizeChromiumPath(strings.TrimPrefix(trimmed, "copy from "))
		case strings.HasPrefix(trimmed, "similarity index "):
			value := strings.TrimSuffix(strings.TrimPrefix(trimmed, "similarity index "), "%")
			if similarity, err := strconv.Atoi(value); err == nil {
				current.Similarity = similarity
			}
		case strings.HasPrefix(trimmed, "Binary files "):
			current.IsBinary = true
			current.Op = OpBinary
		case trimmed == "GIT binary patch":
			current.IsBinary = true
		}
	}

	if err := flush(); err != nil {
		return nil, err
	}
	return patches, nil
}
