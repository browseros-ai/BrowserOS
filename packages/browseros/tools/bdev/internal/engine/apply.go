package engine

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/git"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/patch"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/repo"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/resolve"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/workspace"
)

type ApplyOptions struct {
	Workspace  workspace.Entry
	Repo       *repo.Info
	Reset      bool
	ChangedRef string
	RangeEnd   string
	Filters    []string
	Mode       string
}

type ApplyResult struct {
	Workspace  string              `json:"workspace"`
	Mode       string              `json:"mode"`
	BaseCommit string              `json:"base_commit"`
	RepoRev    string              `json:"repo_rev"`
	Applied    []string            `json:"applied"`
	ResetPaths []string            `json:"reset_paths"`
	Orphaned   []string            `json:"orphaned,omitempty"`
	Conflicts  []resolve.Operation `json:"conflicts,omitempty"`
}

func Apply(ctx context.Context, opts ApplyOptions) (*ApplyResult, error) {
	repoRev, err := git.HeadRev(ctx, opts.Repo.Root)
	if err != nil {
		return nil, err
	}
	ops, orphaned, err := buildApplyOperations(ctx, opts)
	if err != nil {
		return nil, err
	}
	result := &ApplyResult{
		Workspace:  opts.Workspace.Name,
		Mode:       applyMode(opts),
		BaseCommit: opts.Repo.BaseCommit,
		RepoRev:    repoRev,
		Orphaned:   orphaned,
	}
	if len(ops) == 0 {
		if err := markApplyComplete(opts.Workspace.Path, opts.Repo.BaseCommit, repoRev); err != nil {
			return nil, err
		}
		if err := clearResolveState(opts.Workspace.Path); err != nil {
			return nil, err
		}
		return result, nil
	}
	next, err := applyOperationRange(ctx, opts.Workspace, opts.Repo, ops, 0, nil, nil, result)
	if err != nil {
		return nil, err
	}
	if next < len(ops) {
		return result, nil
	}
	if err := markApplyComplete(opts.Workspace.Path, opts.Repo.BaseCommit, repoRev); err != nil {
		return nil, err
	}
	if err := clearResolveState(opts.Workspace.Path); err != nil {
		return nil, err
	}
	return result, nil
}

func Continue(ctx context.Context, ws workspace.Entry) (*ApplyResult, error) {
	state, err := resolve.Load(ws.Path)
	if err != nil {
		return nil, err
	}
	repoInfo, err := repo.Load(state.RepoRoot)
	if err != nil {
		return nil, err
	}
	current, err := state.CurrentOperation()
	if err != nil {
		return nil, err
	}
	if err := verifyResolved(ctx, ws.Path, repoInfo, current, state.BaseCommit); err != nil {
		return nil, err
	}
	state.Resolved = append(state.Resolved, current.ChromiumPath)
	result := &ApplyResult{
		Workspace:  ws.Name,
		Mode:       state.Mode,
		BaseCommit: state.BaseCommit,
		RepoRev:    state.RepoRev,
		Applied:    append([]string{}, state.Resolved...),
		Conflicts:  nil,
	}
	next, err := applyOperationRange(ctx, ws, repoInfo, state.Operations, state.Current+1, state.Resolved, state.Skipped, result)
	if err != nil {
		return nil, err
	}
	if next >= len(state.Operations) && len(result.Conflicts) == 0 {
		if err := markApplyComplete(ws.Path, state.BaseCommit, state.RepoRev); err != nil {
			return nil, err
		}
		if err := resolve.Delete(ws.Path); err != nil {
			return nil, err
		}
	}
	return result, nil
}

func Skip(ctx context.Context, ws workspace.Entry) (*ApplyResult, error) {
	state, err := resolve.Load(ws.Path)
	if err != nil {
		return nil, err
	}
	repoInfo, err := repo.Load(state.RepoRoot)
	if err != nil {
		return nil, err
	}
	current, err := state.CurrentOperation()
	if err != nil {
		return nil, err
	}
	state.Skipped = append(state.Skipped, current.ChromiumPath)
	result := &ApplyResult{
		Workspace:  ws.Name,
		Mode:       state.Mode,
		BaseCommit: state.BaseCommit,
		RepoRev:    state.RepoRev,
		Applied:    append([]string{}, state.Resolved...),
	}
	next, err := applyOperationRange(ctx, ws, repoInfo, state.Operations, state.Current+1, state.Resolved, state.Skipped, result)
	if err != nil {
		return nil, err
	}
	if next >= len(state.Operations) && len(result.Conflicts) == 0 {
		if err := markApplyComplete(ws.Path, state.BaseCommit, state.RepoRev); err != nil {
			return nil, err
		}
		if err := resolve.Delete(ws.Path); err != nil {
			return nil, err
		}
	}
	return result, nil
}

func Abort(ctx context.Context, ws workspace.Entry) error {
	state, err := resolve.Load(ws.Path)
	if err != nil {
		return err
	}
	for idx := 0; idx < len(state.Operations); idx++ {
		op := state.Operations[idx]
		if op.OldPath != "" {
			if err := git.ResetPathToCommit(ctx, ws.Path, state.BaseCommit, op.OldPath); err != nil {
				return err
			}
		}
		if err := git.ResetPathToCommit(ctx, ws.Path, state.BaseCommit, op.ChromiumPath); err != nil {
			return err
		}
		if op.RejectPath != "" {
			_ = os.Remove(op.RejectPath)
		}
	}
	workspaceState, err := workspace.LoadState(ws.Path)
	if err != nil {
		return err
	}
	if err := resolve.Delete(ws.Path); err != nil {
		return err
	}
	if workspaceState.PendingStash == "" {
		return nil
	}
	if err := git.StashPop(ctx, ws.Path, workspaceState.PendingStash); err != nil {
		return err
	}
	workspaceState.PendingStash = ""
	return workspace.SaveState(ws.Path, workspaceState)
}

func buildApplyOperations(ctx context.Context, opts ApplyOptions) ([]resolve.Operation, []string, error) {
	repoSet, err := patch.LoadRepoPatchSet(opts.Repo.PatchesDir, opts.Filters)
	if err != nil {
		return nil, nil, err
	}
	switch {
	case opts.Reset:
		return operationsFromPatchSet(repoSet), nil, nil
	case opts.ChangedRef != "":
		changes, err := repoPatchChanges(ctx, opts.Repo, opts.ChangedRef, opts.RangeEnd)
		if err != nil {
			return nil, nil, err
		}
		return operationsFromChanges(repoSet, changes, opts.Filters), nil, nil
	default:
		localSet, err := patch.BuildWorkingTreePatchSet(ctx, opts.Workspace.Path, opts.Repo.BaseCommit, opts.Filters)
		if err != nil {
			return nil, nil, err
		}
		var ops []resolve.Operation
		var orphaned []string
		for _, delta := range patch.Compare(repoSet, localSet) {
			switch delta.Kind {
			case patch.NeedsApply, patch.NeedsUpdate:
				ops = append(ops, operationFromPatch(*delta.Repo))
			case patch.Orphaned:
				orphaned = append(orphaned, delta.Path)
			}
		}
		return ops, orphaned, nil
	}
}

func applyMode(opts ApplyOptions) string {
	switch {
	case opts.Mode != "":
		return opts.Mode
	case opts.Reset:
		return "reset"
	case opts.ChangedRef != "":
		return "changed"
	default:
		return "incremental"
	}
}

func applyOperationRange(
	ctx context.Context,
	ws workspace.Entry,
	repoInfo *repo.Info,
	ops []resolve.Operation,
	start int,
	resolved []string,
	skipped []string,
	result *ApplyResult,
) (int, error) {
	repoSet, err := patch.LoadRepoPatchSet(repoInfo.PatchesDir, nil)
	if err != nil {
		return start, err
	}
	for idx := start; idx < len(ops); idx++ {
		op := ops[idx]
		result.ResetPaths = append(result.ResetPaths, op.ChromiumPath)
		if op.OldPath != "" {
			if err := git.ResetPathToCommit(ctx, ws.Path, repoInfo.BaseCommit, op.OldPath); err != nil {
				return idx, err
			}
		}
		if err := git.ResetPathToCommit(ctx, ws.Path, repoInfo.BaseCommit, op.ChromiumPath); err != nil {
			return idx, err
		}
		patchFile, ok := repoSet[op.ChromiumPath]
		if ok {
			if err := applySingleOperation(ctx, ws.Path, patchFile); err != nil {
				op.RejectPath = rejectPath(ws.Path, op)
				op.Message = err.Error()
				ops[idx] = op
				state := &resolve.State{
					Workspace:  ws.Path,
					RepoRoot:   repoInfo.Root,
					BaseCommit: repoInfo.BaseCommit,
					RepoRev:    result.RepoRev,
					Mode:       result.Mode,
					Current:    idx,
					Operations: ops,
					Resolved:   append([]string{}, resolved...),
					Skipped:    append([]string{}, skipped...),
				}
				if err := resolve.Save(ws.Path, state); err != nil {
					return idx, err
				}
				result.Conflicts = []resolve.Operation{op}
				return idx, nil
			}
		} else if op.Op == patch.OpDelete {
			if err := os.RemoveAll(filepath.Join(ws.Path, filepath.FromSlash(op.ChromiumPath))); err != nil {
				return idx, err
			}
		}
		result.Applied = append(result.Applied, op.ChromiumPath)
	}
	return len(ops), nil
}

func applySingleOperation(ctx context.Context, workspacePath string, patchFile patch.FilePatch) error {
	switch {
	case patchFile.Op == patch.OpDelete:
		return os.RemoveAll(filepath.Join(workspacePath, filepath.FromSlash(patchFile.Path)))
	case patchFile.IsPureRename():
		from := filepath.Join(workspacePath, filepath.FromSlash(patchFile.OldPath))
		to := filepath.Join(workspacePath, filepath.FromSlash(patchFile.Path))
		if err := os.MkdirAll(filepath.Dir(to), 0o755); err != nil {
			return err
		}
		return os.Rename(from, to)
	case patchFile.Op == patch.OpBinary && len(patchFile.Content) == 0:
		return fmt.Errorf("binary markers are not directly applicable: %s", patchFile.Path)
	default:
		_, err := git.ApplyPatch(ctx, workspacePath, patchFile.Content)
		return err
	}
}

func verifyResolved(ctx context.Context, workspacePath string, repoInfo *repo.Info, op resolve.Operation, base string) error {
	repoSet, err := patch.LoadRepoPatchSet(repoInfo.PatchesDir, []string{op.ChromiumPath})
	if err != nil {
		return err
	}
	localSet, err := patch.BuildWorkingTreePatchSet(ctx, workspacePath, base, []string{op.ChromiumPath})
	if err != nil {
		return err
	}
	for _, delta := range patch.Compare(repoSet, localSet) {
		if delta.Path == op.ChromiumPath && delta.Kind == patch.UpToDate {
			if op.RejectPath != "" {
				_ = os.Remove(op.RejectPath)
			}
			return nil
		}
	}
	return fmt.Errorf("current conflict is not resolved yet for %s", op.ChromiumPath)
}

func operationFromPatch(p patch.FilePatch) resolve.Operation {
	return resolve.Operation{
		ChromiumPath: p.Path,
		PatchRel:     p.Path,
		Op:           p.Op,
		OldPath:      p.OldPath,
	}
}

func operationsFromPatchSet(set patch.PatchSet) []resolve.Operation {
	paths := patch.ScopeFromSet(set)
	ops := make([]resolve.Operation, 0, len(paths))
	for _, rel := range paths {
		ops = append(ops, operationFromPatch(set[rel]))
	}
	return ops
}

func operationsFromChanges(repoSet patch.PatchSet, changes []git.FileChange, filters []string) []resolve.Operation {
	var ops []resolve.Operation
	for _, change := range changes {
		rel := normalizeChangedPatchPath(change.Path)
		if !patch.PathMatches(rel, filters) {
			continue
		}
		if patchFile, ok := repoSet[rel]; ok {
			ops = append(ops, operationFromPatch(patchFile))
			continue
		}
		ops = append(ops, resolve.Operation{
			ChromiumPath: rel,
			PatchRel:     rel,
			Op:           patch.OpDelete,
			OldPath:      normalizeChangedPatchPath(change.OldPath),
		})
	}
	return ops
}

func repoPatchChanges(ctx context.Context, repoInfo *repo.Info, ref string, rangeEnd string) ([]git.FileChange, error) {
	pathspecs := []string{"chromium_patches"}
	if rangeEnd == "" {
		return git.DiffTreeNameStatus(ctx, repoInfo.Root, ref, pathspecs)
	}
	return git.DiffNameStatusBetween(ctx, repoInfo.Root, ref, rangeEnd, pathspecs)
}

func rejectPath(workspacePath string, op resolve.Operation) string {
	candidate := patch.RejectPath(workspacePath, op.ChromiumPath)
	if _, err := os.Stat(candidate); err == nil {
		return candidate
	}
	return ""
}

func normalizeChangedPatchPath(path string) string {
	return strings.TrimPrefix(patch.NormalizeChromiumPath(path), "chromium_patches/")
}

func clearResolveState(workspacePath string) error {
	if resolve.Exists(workspacePath) {
		return resolve.Delete(workspacePath)
	}
	return nil
}

func markApplyComplete(workspacePath string, baseCommit string, repoRev string) error {
	state, err := workspace.LoadState(workspacePath)
	if err != nil {
		return err
	}
	state.BaseCommit = baseCommit
	state.LastApplyRev = repoRev
	state.LastApplyAt = time.Now().UTC()
	return workspace.SaveState(workspacePath, state)
}
