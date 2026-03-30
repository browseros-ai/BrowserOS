package git

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

type Result struct {
	Stdout string
	Stderr string
	Code   int
}

type FileChange struct {
	Status  string `json:"status"`
	Path    string `json:"path"`
	OldPath string `json:"old_path,omitempty"`
}

func Run(ctx context.Context, dir string, stdin []byte, args ...string) (Result, error) {
	command := exec.CommandContext(ctx, "git", args...)
	command.Dir = dir
	if stdin != nil {
		command.Stdin = bytes.NewReader(stdin)
	}
	var stdout bytes.Buffer
	var stderr bytes.Buffer
	command.Stdout = &stdout
	command.Stderr = &stderr
	err := command.Run()
	code := -1
	if command.ProcessState != nil {
		code = command.ProcessState.ExitCode()
	}
	result := Result{
		Stdout: stdout.String(),
		Stderr: stderr.String(),
		Code:   code,
	}
	if err == nil {
		return result, nil
	}
	if errors.Is(err, context.DeadlineExceeded) || errors.Is(ctx.Err(), context.DeadlineExceeded) {
		return result, err
	}
	if errors.Is(err, context.Canceled) || errors.Is(ctx.Err(), context.Canceled) {
		return result, err
	}
	if command.ProcessState == nil {
		return result, err
	}
	return result, nil
}

func HeadRev(ctx context.Context, dir string) (string, error) {
	result, err := Run(ctx, dir, nil, "rev-parse", "HEAD")
	if err != nil {
		return "", err
	}
	if result.Code != 0 {
		return "", errors.New(strings.TrimSpace(result.Stderr))
	}
	return strings.TrimSpace(result.Stdout), nil
}

func CurrentBranch(ctx context.Context, dir string) (string, error) {
	result, err := Run(ctx, dir, nil, "branch", "--show-current")
	if err != nil {
		return "", err
	}
	if result.Code != 0 {
		return "", errors.New(strings.TrimSpace(result.Stderr))
	}
	return strings.TrimSpace(result.Stdout), nil
}

func IsDirty(ctx context.Context, dir string) (bool, error) {
	return IsDirtyPaths(ctx, dir, nil)
}

func IsDirtyPaths(ctx context.Context, dir string, pathspecs []string) (bool, error) {
	args := []string{"status", "--porcelain"}
	if len(pathspecs) > 0 {
		args = append(args, "--")
		args = append(args, pathspecs...)
	}
	result, err := Run(ctx, dir, nil, args...)
	if err != nil {
		return false, err
	}
	if result.Code != 0 {
		return false, errors.New(strings.TrimSpace(result.Stderr))
	}
	return strings.TrimSpace(result.Stdout) != "", nil
}

func CommitExists(ctx context.Context, dir string, ref string) (bool, error) {
	result, err := Run(ctx, dir, nil, "rev-parse", "--verify", ref+"^{commit}")
	if err != nil {
		return false, err
	}
	return result.Code == 0, nil
}

func FileExistsAtCommit(ctx context.Context, dir string, ref string, rel string) (bool, error) {
	result, err := Run(ctx, dir, nil, "cat-file", "-e", fmt.Sprintf("%s:%s", ref, rel))
	if err != nil {
		return false, err
	}
	return result.Code == 0, nil
}

func ShowFile(ctx context.Context, dir string, ref string, rel string) ([]byte, error) {
	result, err := Run(ctx, dir, nil, "show", fmt.Sprintf("%s:%s", ref, rel))
	if err != nil {
		return nil, err
	}
	if result.Code != 0 {
		return nil, errors.New(strings.TrimSpace(result.Stderr))
	}
	return []byte(result.Stdout), nil
}

func CheckoutFiles(ctx context.Context, dir string, ref string, paths []string) error {
	if len(paths) == 0 {
		return nil
	}
	args := []string{"checkout", ref, "--"}
	args = append(args, paths...)
	result, err := Run(ctx, dir, nil, args...)
	if err != nil {
		return err
	}
	if result.Code != 0 {
		return errors.New(strings.TrimSpace(result.Stderr))
	}
	return nil
}

func ResetPathToCommit(ctx context.Context, dir string, ref string, rel string) error {
	exists, err := FileExistsAtCommit(ctx, dir, ref, rel)
	if err != nil {
		return err
	}
	target := filepath.Join(dir, filepath.FromSlash(rel))
	if exists {
		return CheckoutFiles(ctx, dir, ref, []string{rel})
	}
	return os.RemoveAll(target)
}

func DiffText(ctx context.Context, dir string, args ...string) (string, error) {
	result, err := Run(ctx, dir, nil, append([]string{"diff", "--binary", "-M"}, args...)...)
	if err != nil {
		return "", err
	}
	if result.Code != 0 {
		return "", errors.New(strings.TrimSpace(result.Stderr))
	}
	return result.Stdout, nil
}

func DiffNoIndex(ctx context.Context, dir string, path string) (string, error) {
	result, err := Run(ctx, dir, nil, "diff", "--binary", "--no-index", "--", "/dev/null", path)
	if err != nil {
		return "", err
	}
	if result.Code != 0 && result.Code != 1 {
		return "", errors.New(strings.TrimSpace(result.Stderr))
	}
	return result.Stdout, nil
}

func ListUntracked(ctx context.Context, dir string, pathspecs []string) ([]string, error) {
	args := []string{"ls-files", "--others", "--exclude-standard"}
	if len(pathspecs) > 0 {
		args = append(args, "--")
		args = append(args, pathspecs...)
	}
	result, err := Run(ctx, dir, nil, args...)
	if err != nil {
		return nil, err
	}
	if result.Code != 0 {
		return nil, errors.New(strings.TrimSpace(result.Stderr))
	}
	lines := splitLines(result.Stdout)
	return lines, nil
}

func DiffNameStatusBetween(ctx context.Context, dir string, from string, to string, pathspecs []string) ([]FileChange, error) {
	args := []string{"diff", "--name-status", "-M", fmt.Sprintf("%s..%s", from, to)}
	if len(pathspecs) > 0 {
		args = append(args, "--")
		args = append(args, pathspecs...)
	}
	return runNameStatus(ctx, dir, args...)
}

func DiffTreeNameStatus(ctx context.Context, dir string, ref string, pathspecs []string) ([]FileChange, error) {
	args := []string{"diff-tree", "--no-commit-id", "--name-status", "-r", ref}
	if len(pathspecs) > 0 {
		args = append(args, "--")
		args = append(args, pathspecs...)
	}
	return runNameStatus(ctx, dir, args...)
}

func RevListRange(ctx context.Context, dir string, start string, end string) ([]string, error) {
	result, err := Run(ctx, dir, nil, "rev-list", "--reverse", fmt.Sprintf("%s..%s", start, end))
	if err != nil {
		return nil, err
	}
	if result.Code != 0 {
		return nil, errors.New(strings.TrimSpace(result.Stderr))
	}
	return splitLines(result.Stdout), nil
}

func ApplyPatch(ctx context.Context, dir string, patch []byte) (string, error) {
	strategies := [][]string{
		{"apply", "--ignore-whitespace", "--whitespace=nowarn", "-p1"},
		{"apply", "--ignore-whitespace", "--whitespace=nowarn", "-p1", "--3way"},
		{"apply", "--ignore-whitespace", "--whitespace=fix", "-p1"},
		{"apply", "--reject", "--ignore-whitespace", "--whitespace=nowarn", "-p1"},
	}
	var lastErr string
	for _, args := range strategies {
		result, err := Run(ctx, dir, patch, args...)
		if err != nil {
			return "", err
		}
		if result.Code == 0 {
			return strings.Join(args[1:], " "), nil
		}
		lastErr = strings.TrimSpace(result.Stderr)
	}
	if lastErr == "" {
		lastErr = "git apply failed"
	}
	return "", errors.New(lastErr)
}

func StashPush(ctx context.Context, dir string, message string, includeUntracked bool, paths []string) (string, error) {
	args := []string{"stash", "push", "-m", message}
	if includeUntracked {
		args = append(args, "-u")
	}
	if len(paths) > 0 {
		args = append(args, "--")
		args = append(args, paths...)
	}
	result, err := Run(ctx, dir, nil, args...)
	if err != nil {
		return "", err
	}
	if result.Code != 0 {
		return "", errors.New(strings.TrimSpace(result.Stderr))
	}
	if strings.Contains(result.Stdout, "No local changes to save") {
		return "", nil
	}
	list, err := Run(ctx, dir, nil, "stash", "list", "-1", "--format=%gd")
	if err != nil {
		return "", err
	}
	if list.Code != 0 {
		return "", errors.New(strings.TrimSpace(list.Stderr))
	}
	return strings.TrimSpace(list.Stdout), nil
}

func StashPop(ctx context.Context, dir string, stashRef string) error {
	args := []string{"stash", "pop"}
	if stashRef != "" {
		args = append(args, stashRef)
	}
	result, err := Run(ctx, dir, nil, args...)
	if err != nil {
		return err
	}
	if result.Code != 0 {
		return errors.New(strings.TrimSpace(result.Stderr))
	}
	return nil
}

func PullRebase(ctx context.Context, dir string, remote string, branch string) error {
	args := []string{"pull", "--rebase"}
	if remote != "" {
		args = append(args, remote)
		if branch != "" {
			args = append(args, branch)
		}
	}
	result, err := Run(ctx, dir, nil, args...)
	if err != nil {
		return err
	}
	if result.Code != 0 {
		return errors.New(strings.TrimSpace(result.Stderr))
	}
	return nil
}

func AddPaths(ctx context.Context, dir string, paths []string) error {
	if len(paths) == 0 {
		return nil
	}
	args := append([]string{"add", "--"}, paths...)
	result, err := Run(ctx, dir, nil, args...)
	if err != nil {
		return err
	}
	if result.Code != 0 {
		return errors.New(strings.TrimSpace(result.Stderr))
	}
	return nil
}

func Commit(ctx context.Context, dir string, message string) error {
	result, err := Run(ctx, dir, nil, "commit", "-m", message)
	if err != nil {
		return err
	}
	if result.Code != 0 {
		return errors.New(strings.TrimSpace(result.Stderr))
	}
	return nil
}

func Push(ctx context.Context, dir string, remote string, branch string) error {
	args := []string{"push"}
	if remote != "" {
		args = append(args, remote)
	}
	if branch != "" {
		args = append(args, branch)
	}
	result, err := Run(ctx, dir, nil, args...)
	if err != nil {
		return err
	}
	if result.Code != 0 {
		return errors.New(strings.TrimSpace(result.Stderr))
	}
	return nil
}

func runNameStatus(ctx context.Context, dir string, args ...string) ([]FileChange, error) {
	result, err := Run(ctx, dir, nil, args...)
	if err != nil {
		return nil, err
	}
	if result.Code != 0 {
		return nil, errors.New(strings.TrimSpace(result.Stderr))
	}
	var changes []FileChange
	for _, line := range splitLines(result.Stdout) {
		parts := strings.Split(line, "\t")
		if len(parts) < 2 {
			continue
		}
		change := FileChange{Status: parts[0][:1], Path: parts[len(parts)-1]}
		if change.Status == "R" || change.Status == "C" {
			if len(parts) >= 3 {
				change.OldPath = parts[1]
			}
		}
		changes = append(changes, change)
	}
	return changes, nil
}

func splitLines(raw string) []string {
	lines := strings.Split(strings.TrimSpace(raw), "\n")
	if len(lines) == 1 && lines[0] == "" {
		return nil
	}
	return lines
}
