package patch

import (
	"bytes"
	"strings"
)

type Delta struct {
	NeedsUpdate []string // In both, but content differs
	NeedsApply  []string // In repo only
	UpToDate    []string // In both, content matches
	Orphaned    []string // Local only, no repo patch
	Deleted     []string // .deleted markers in repo
}

// Compare computes the delta between local patch set and repo patch set.
func Compare(local, repo *PatchSet) *Delta {
	d := &Delta{}

	for path, repoPatch := range repo.Patches {
		if repoPatch.Op == OpDeleted {
			d.Deleted = append(d.Deleted, path)
			continue
		}

		localPatch, exists := local.Patches[path]
		if !exists {
			d.NeedsApply = append(d.NeedsApply, path)
			continue
		}

		if patchContentEqual(localPatch.Content, repoPatch.Content) {
			d.UpToDate = append(d.UpToDate, path)
		} else {
			d.NeedsUpdate = append(d.NeedsUpdate, path)
		}
	}

	for path := range local.Patches {
		if _, exists := repo.Patches[path]; !exists {
			d.Orphaned = append(d.Orphaned, path)
		}
	}

	return d
}

func patchContentEqual(a, b []byte) bool {
	return bytes.Equal(
		normalizePatch(a),
		normalizePatch(b),
	)
}

func normalizePatch(content []byte) []byte {
	lines := strings.Split(string(content), "\n")
	var normalized []string
	for _, line := range lines {
		// Skip index lines (they contain hashes that may differ)
		if strings.HasPrefix(line, "index ") {
			continue
		}
		normalized = append(normalized, strings.TrimRight(line, " \t"))
	}
	return []byte(strings.Join(normalized, "\n"))
}
