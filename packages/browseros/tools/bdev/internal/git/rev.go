package git

import "fmt"

func RevParse(dir, ref string) (string, error) {
	out, err := Run(dir, "rev-parse", ref)
	if err != nil {
		return "", fmt.Errorf("rev-parse %s: %w", ref, err)
	}
	return out, nil
}

func CommitExists(dir, commit string) bool {
	_, err := Run(dir, "cat-file", "-e", commit+"^{commit}")
	return err == nil
}

func HeadRev(dir string) (string, error) {
	return RevParse(dir, "HEAD")
}

// FileExistsInCommit checks whether a file path exists in a given commit.
// Uses git cat-file -e {commit}:{path}.
func FileExistsInCommit(dir, commit, filePath string) bool {
	_, err := Run(dir, "cat-file", "-e", commit+":"+filePath)
	return err == nil
}
