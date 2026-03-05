package git

import (
	"fmt"
	"strings"
)

func ListRemotes(dir string) ([]string, error) {
	out, err := Run(dir, "remote")
	if err != nil {
		return nil, fmt.Errorf("listing remotes: %w", err)
	}
	if strings.TrimSpace(out) == "" {
		return nil, nil
	}

	var remotes []string
	for _, line := range strings.Split(out, "\n") {
		line = strings.TrimSpace(line)
		if line != "" {
			remotes = append(remotes, line)
		}
	}
	return remotes, nil
}

func HasRemote(dir, remote string) (bool, error) {
	remotes, err := ListRemotes(dir)
	if err != nil {
		return false, err
	}
	for _, name := range remotes {
		if name == remote {
			return true, nil
		}
	}
	return false, nil
}

func CurrentBranch(dir string) (branch string, detached bool, err error) {
	out, runErr := Run(dir, "symbolic-ref", "--quiet", "--short", "HEAD")
	if runErr != nil {
		return "", true, nil
	}

	branch = strings.TrimSpace(out)
	if branch == "" {
		return "", true, nil
	}
	return branch, false, nil
}

func Fetch(dir, remote string) error {
	_, err := Run(dir, "fetch", "--prune", remote)
	if err != nil {
		return fmt.Errorf("fetch %s: %w", remote, err)
	}
	return nil
}

func Pull(dir, remote, branch string, rebase bool) error {
	args := []string{"pull"}
	if rebase {
		args = append(args, "--rebase")
	} else {
		args = append(args, "--ff-only")
	}
	args = append(args, remote)
	if strings.TrimSpace(branch) != "" {
		args = append(args, branch)
	}

	_, err := Run(dir, args...)
	if err != nil {
		return fmt.Errorf("pull %s/%s: %w", remote, branch, err)
	}
	return nil
}

func Push(dir, remote, branch string) error {
	args := []string{"push", remote}
	if strings.TrimSpace(branch) != "" {
		args = append(args, "HEAD:"+branch)
	}

	_, err := Run(dir, args...)
	if err != nil {
		return fmt.Errorf("push %s/%s: %w", remote, branch, err)
	}
	return nil
}
