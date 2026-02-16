package main

import (
	"errors"
	"fmt"
	"os"

	"bros/cmd"
	"bros/internal/exitcode"
)

var version = "dev"

func main() {
	cmd.SetVersion(version)
	if err := cmd.Execute(); err != nil {
		var codedErr *exitcode.Error
		if errors.As(err, &codedErr) {
			if codedErr.HasMessage() {
				fmt.Fprintln(os.Stderr, codedErr.Error())
			}
			os.Exit(codedErr.ExitCode())
		}

		fmt.Fprintln(os.Stderr, err)
		os.Exit(2)
	}
}
