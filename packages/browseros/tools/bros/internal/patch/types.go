package patch

type FileOp int

const (
	OpModified FileOp = iota
	OpAdded
	OpDeleted
	OpRenamed
	OpBinary
)

func (op FileOp) String() string {
	switch op {
	case OpModified:
		return "M"
	case OpAdded:
		return "A"
	case OpDeleted:
		return "D"
	case OpRenamed:
		return "R"
	case OpBinary:
		return "B"
	default:
		return "?"
	}
}

type FilePatch struct {
	Path       string
	Op         FileOp
	Content    []byte
	OldPath    string // for renames
	Similarity int    // for renames
	IsBinary   bool
}

type PatchSet struct {
	Base    string
	Patches map[string]*FilePatch // keyed by chromium path
}

func NewPatchSet(base string) *PatchSet {
	return &PatchSet{
		Base:    base,
		Patches: make(map[string]*FilePatch),
	}
}

type PushResult struct {
	Modified  []string
	Added     []string
	Deleted   []string
	Stale     []string
	Unchanged []string
}

func (r *PushResult) Total() int {
	return len(r.Modified) + len(r.Added) + len(r.Deleted)
}

type PullResult struct {
	Applied   []string
	Skipped   []string
	Conflicts []ConflictInfo
	Deleted   []string
}

type ConflictInfo struct {
	File        string
	RejectFile  string
	PatchFile   string
	HunksTotal  int
	HunksFailed int
	Error       string
}
