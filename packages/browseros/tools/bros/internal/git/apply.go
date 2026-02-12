package git

import (
	"bytes"
	"context"
	"fmt"
	"os"
	"os/exec"
	"regexp"
	"strconv"
	"strings"
	"time"

	"bros/internal/patch"
)

var rejHunkRe = regexp.MustCompile(`Applying patch .* with (\d+) hunks?`)
var rejFailRe = regexp.MustCompile(`(\d+) out of (\d+) hunks? FAILED`)

// Apply tries multiple strategies to apply a patch, falling back to --reject
// only as a last resort. Mirrors the Python CLI fallback chain.
func Apply(dir string, patchContent []byte, patchFile string) (*patch.ConflictInfo, error) {
	// Strategy 1: Clean apply
	if tryApply(dir, patchContent, "-p1") == nil {
		return nil, nil
	}

	// Strategy 2: Three-way merge
	if tryApply(dir, patchContent, "-p1", "--3way") == nil {
		return nil, nil
	}

	// Strategy 3: Whitespace fix
	if tryApply(dir, patchContent, "-p1", "--whitespace=fix") == nil {
		return nil, nil
	}

	// Strategy 4: Reject (last resort — partially applies, creates .rej files)
	return applyReject(dir, patchContent, patchFile)
}

// tryApply attempts a git apply with the given flags. Returns nil on success.
func tryApply(dir string, patchContent []byte, flags ...string) error {
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()

	args := append([]string{"apply"}, flags...)
	cmd := exec.CommandContext(ctx, "git", args...)
	cmd.Dir = dir
	cmd.Stdin = bytes.NewReader(patchContent)

	var stderr bytes.Buffer
	cmd.Stderr = &stderr

	return cmd.Run()
}

// applyReject applies a patch with --reject, creating .rej files for failed hunks.
func applyReject(dir string, patchContent []byte, patchFile string) (*patch.ConflictInfo, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()

	cmd := exec.CommandContext(ctx, "git", "apply",
		"--reject",
		"--ignore-whitespace",
		"--whitespace=nowarn",
		"-p1",
	)
	cmd.Dir = dir
	cmd.Stdin = bytes.NewReader(patchContent)

	var stderr bytes.Buffer
	cmd.Stderr = &stderr

	err := cmd.Run()
	if err == nil {
		return nil, nil
	}

	stderrStr := stderr.String()
	info := &patch.ConflictInfo{
		PatchFile: patchFile,
		Error:     strings.TrimSpace(stderrStr),
	}

	if m := rejFailRe.FindStringSubmatch(stderrStr); len(m) == 3 {
		info.HunksFailed, _ = strconv.Atoi(m[1])
		info.HunksTotal, _ = strconv.Atoi(m[2])
	} else if m := rejHunkRe.FindStringSubmatch(stderrStr); len(m) == 2 {
		info.HunksTotal, _ = strconv.Atoi(m[1])
		info.HunksFailed = info.HunksTotal
	}

	return info, nil
}

// ApplyPatchFile applies a patch from a file path using the full fallback chain.
func ApplyPatchFile(dir, patchPath string) (*patch.ConflictInfo, error) {
	content, err := os.ReadFile(patchPath)
	if err != nil {
		return nil, fmt.Errorf("reading patch file %s: %w", patchPath, err)
	}
	info, err := Apply(dir, content, patchPath)
	if info != nil {
		info.PatchFile = patchPath
	}
	return info, err
}

// ApplyCheck tests if a patch would apply without modifying anything.
func ApplyCheck(dir string, patchContent []byte) error {
	cmd := exec.Command("git", "apply", "--check", "-p1")
	cmd.Dir = dir
	cmd.Stdin = bytes.NewReader(patchContent)

	var stderr bytes.Buffer
	cmd.Stderr = &stderr

	if err := cmd.Run(); err != nil {
		return fmt.Errorf("patch would not apply cleanly: %s", stderr.String())
	}
	return nil
}
