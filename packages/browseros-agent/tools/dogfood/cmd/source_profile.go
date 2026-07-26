package cmd

import (
	"bufio"
	"fmt"
	"io"
	"strings"

	"browseros-dogfood/config"
	"browseros-dogfood/profile"
)

func promptIfSourceProfileInUse(out io.Writer, r *bufio.Reader, cfg config.Config, refreshProfile bool) error {
	if !profileImportNeeded(cfg, refreshProfile) {
		return nil
	}
	return confirmSourceProfileImport(out, r, cfg.SourceUserDataDir)
}

func profileImportNeeded(cfg config.Config, refreshProfile bool) bool {
	return refreshProfile || !exists(cfg.DevUserDataDir)
}

func confirmSourceProfileImport(out io.Writer, r *bufio.Reader, sourceUserDataDir string) error {
	return confirmSourceProfileImportWithChecker(out, r, func() (bool, error) {
		return profile.HasSingletons(sourceUserDataDir)
	})
}

func confirmSourceProfileImportWithChecker(out io.Writer, r *bufio.Reader, hasSingletons func() (bool, error)) error {
	for {
		active, err := hasSingletons()
		if err != nil {
			return err
		}
		if !active {
			return nil
		}
		fmt.Fprintln(out, warnStyle.Sprint("BrowserOS appears to be using the source profile."))
		fmt.Fprintf(out, "%s ", labelStyle.Sprint("Quit BrowserOS, then press Enter to retry, or type \"continue\" to import anyway:"))
		line, err := r.ReadString('\n')
		if strings.EqualFold(strings.TrimSpace(line), "continue") {
			return nil
		}
		if err != nil {
			return err
		}
	}
}
