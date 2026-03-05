package patch

import (
	"bytes"
	"fmt"
	"strings"
)

var diffHeaderPrefix = []byte("diff --git ")

// ParseUnifiedDiff parses a full `git diff` output into individual file patches.
func ParseUnifiedDiff(raw []byte) (*PatchSet, error) {
	ps := NewPatchSet("")

	chunks := splitDiffChunks(raw)
	for _, chunk := range chunks {
		fp, err := parseSingleDiff(chunk)
		if err != nil {
			return nil, fmt.Errorf("parsing diff chunk: %w", err)
		}
		if fp != nil {
			ps.Patches[fp.Path] = fp
		}
	}

	return ps, nil
}

func splitDiffChunks(raw []byte) [][]byte {
	var chunks [][]byte
	lines := bytes.Split(raw, []byte("\n"))

	var current [][]byte
	for _, line := range lines {
		if bytes.HasPrefix(line, diffHeaderPrefix) {
			if len(current) > 0 {
				chunks = append(chunks, bytes.Join(current, []byte("\n")))
			}
			current = [][]byte{line}
		} else if len(current) > 0 {
			current = append(current, line)
		}
	}
	if len(current) > 0 {
		chunks = append(chunks, bytes.Join(current, []byte("\n")))
	}

	return chunks
}

func parseSingleDiff(chunk []byte) (*FilePatch, error) {
	lines := strings.Split(string(chunk), "\n")
	if len(lines) == 0 {
		return nil, nil
	}

	fp := &FilePatch{
		Op:      OpModified,
		Content: chunk,
	}

	// Parse the diff --git a/... b/... header
	header := lines[0]
	if !strings.HasPrefix(header, "diff --git ") {
		return nil, fmt.Errorf("unexpected header: %s", header)
	}

	// Extract b/ path from the header
	parts := strings.SplitN(header, " b/", 2)
	if len(parts) == 2 {
		fp.Path = parts[1]
	}

	// Scan header lines for operation type
	for _, line := range lines[1:] {
		if strings.HasPrefix(line, "diff --git ") || strings.HasPrefix(line, "@@") {
			break
		}

		switch {
		case strings.HasPrefix(line, "new file mode"):
			fp.Op = OpAdded
		case strings.HasPrefix(line, "deleted file mode"):
			fp.Op = OpDeleted
		case strings.HasPrefix(line, "rename from "):
			fp.Op = OpRenamed
			fp.OldPath = strings.TrimPrefix(line, "rename from ")
		case strings.HasPrefix(line, "rename to "):
			fp.Path = strings.TrimPrefix(line, "rename to ")
		case strings.HasPrefix(line, "similarity index "):
			s := strings.TrimPrefix(line, "similarity index ")
			s = strings.TrimSuffix(s, "%")
			fmt.Sscanf(s, "%d", &fp.Similarity)
		case strings.Contains(line, "Binary files"):
			fp.Op = OpBinary
			fp.IsBinary = true
		}
	}

	if fp.Path == "" {
		return nil, nil
	}

	return fp, nil
}
