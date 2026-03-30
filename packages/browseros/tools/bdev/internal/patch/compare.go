package patch

import (
	"slices"
	"strings"
)

func Compare(repo PatchSet, local PatchSet) []Delta {
	seen := map[string]bool{}
	var deltas []Delta
	for rel, repoPatch := range repo {
		seen[rel] = true
		localPatch, ok := local[rel]
		switch {
		case !ok:
			deltas = append(deltas, Delta{Path: rel, Kind: NeedsApply, Repo: ptr(repoPatch)})
		case signature(repoPatch) == signature(localPatch):
			deltas = append(deltas, Delta{Path: rel, Kind: UpToDate, Repo: ptr(repoPatch), Local: ptr(localPatch)})
		default:
			deltas = append(deltas, Delta{Path: rel, Kind: NeedsUpdate, Repo: ptr(repoPatch), Local: ptr(localPatch)})
		}
	}
	for rel, localPatch := range local {
		if seen[rel] {
			continue
		}
		deltas = append(deltas, Delta{Path: rel, Kind: Orphaned, Local: ptr(localPatch)})
	}
	slices.SortFunc(deltas, func(a, b Delta) int {
		if a.Path < b.Path {
			return -1
		}
		if a.Path > b.Path {
			return 1
		}
		return 0
	})
	return deltas
}

func signature(p FilePatch) string {
	switch {
	case p.Op == OpDelete:
		return "delete:" + NormalizeChromiumPath(p.Path)
	case p.IsPureRename():
		return strings.Join([]string{"rename", NormalizeChromiumPath(p.OldPath), NormalizeChromiumPath(p.Path)}, ":")
	case p.Op == OpBinary && len(p.Content) == 0:
		return "binary:" + NormalizeChromiumPath(p.Path)
	default:
		lines := strings.Split(strings.ReplaceAll(string(p.Content), "\r\n", "\n"), "\n")
		normalized := make([]string, 0, len(lines))
		for _, line := range lines {
			if strings.HasPrefix(line, "index ") {
				continue
			}
			normalized = append(normalized, strings.TrimRight(line, " \t"))
		}
		return strings.Join(append([]string{string(p.Op), NormalizeChromiumPath(p.Path), NormalizeChromiumPath(p.OldPath)}, normalized...), "\n")
	}
}

func ptr(p FilePatch) *FilePatch {
	copy := p
	return &copy
}
