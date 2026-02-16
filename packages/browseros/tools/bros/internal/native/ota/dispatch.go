package ota

import (
	"fmt"
	"strings"
)

// Run handles native OTA subcommands. The bool return indicates whether the
// command was recognized and handled natively.
func Run(args []string, packagesDir string) (bool, error) {
	if len(args) == 0 || strings.TrimSpace(args[0]) != "ota" {
		return false, nil
	}
	if len(args) < 2 {
		return false, nil
	}

	switch strings.TrimSpace(args[1]) {
	case "test-signing":
		return true, runTestSigning(args[2:], packagesDir)
	case "server":
		if len(args) < 3 {
			return false, nil
		}
		sub := strings.TrimSpace(args[2])
		subArgs := args[3:]

		switch sub {
		case "release":
			return true, runServerRelease(subArgs, packagesDir)
		case "release-appcast", "publish-appcast":
			return true, runServerPublishAppcast(subArgs, packagesDir)
		case "list-platforms":
			if len(subArgs) > 0 {
				return true, fmt.Errorf("list-platforms does not accept arguments")
			}
			return true, runListPlatforms()
		default:
			return false, nil
		}
	default:
		return false, nil
	}
}
