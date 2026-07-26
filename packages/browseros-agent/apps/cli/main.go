package main

import "browseros-cli/cmd"

var (
	version        = "dev"
	buildTimestamp string
)

func main() {
	cmd.SetVersion(resolvedVersion(version, buildTimestamp))
	cmd.Execute()
}

// resolvedVersion adds build-time metadata only to local dev versions.
func resolvedVersion(version, buildTimestamp string) string {
	if version == "dev" && buildTimestamp != "" {
		return version + "-" + buildTimestamp
	}
	return version
}
