package patch

import (
	"bytes"
	"path"
	"path/filepath"
	"strings"
)

type FileOp string

const (
	OpAdd    FileOp = "ADD"
	OpModify FileOp = "MODIFY"
	OpDelete FileOp = "DELETE"
	OpRename FileOp = "RENAME"
	OpCopy   FileOp = "COPY"
	OpBinary FileOp = "BINARY"
)

type FilePatch struct {
	Path       string `json:"path"`
	Op         FileOp `json:"op"`
	OldPath    string `json:"old_path,omitempty"`
	Similarity int    `json:"similarity,omitempty"`
	Content    []byte `json:"-"`
	IsBinary   bool   `json:"is_binary,omitempty"`
}

type PatchSet map[string]FilePatch

type DeltaKind string

const (
	NeedsApply  DeltaKind = "needs_apply"
	NeedsUpdate DeltaKind = "needs_update"
	UpToDate    DeltaKind = "up_to_date"
	Orphaned    DeltaKind = "orphaned"
)

type Delta struct {
	Path  string     `json:"path"`
	Kind  DeltaKind  `json:"kind"`
	Repo  *FilePatch `json:"repo,omitempty"`
	Local *FilePatch `json:"local,omitempty"`
}

func NormalizeChromiumPath(raw string) string {
	clean := filepath.ToSlash(raw)
	return strings.TrimPrefix(path.Clean(clean), "./")
}

func PathMatches(rel string, filters []string) bool {
	if IsInternalPath(rel) {
		return false
	}
	if len(filters) == 0 {
		return true
	}
	candidate := NormalizeChromiumPath(rel)
	for _, filter := range filters {
		scope := NormalizeChromiumPath(filter)
		if candidate == scope || strings.HasPrefix(candidate, scope+"/") {
			return true
		}
	}
	return false
}

func IsInternalPath(rel string) bool {
	candidate := NormalizeChromiumPath(rel)
	return candidate == ".bdev" || strings.HasPrefix(candidate, ".bdev/")
}

func (p FilePatch) IsPureRename() bool {
	if p.Op != OpRename {
		return false
	}
	return !bytes.Contains(p.Content, []byte("\n@@")) && !bytes.Contains(p.Content, []byte("GIT binary patch"))
}
