package git

import "fmt"

// CheckoutFiles resets multiple files to a specific commit.
// Batches into a single git call.
func CheckoutFiles(dir, commit string, files []string) error {
	if len(files) == 0 {
		return nil
	}

	args := []string{"checkout", commit, "--"}
	args = append(args, files...)

	_, err := Run(dir, args...)
	if err != nil {
		return fmt.Errorf("checkout %s -- [%d files]: %w", commit, len(files), err)
	}
	return nil
}
