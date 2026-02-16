package dev

import (
	"fmt"
	"strings"
)

type addUpdateFlags struct {
	Name        string
	Commit      string
	Description string
}

func parseGlobalFlags(args []string) (Options, []string, error) {
	var opts Options

	i := 0
	for i < len(args) {
		arg := args[i]
		if !strings.HasPrefix(arg, "-") {
			break
		}

		switch {
		case arg == "--chromium-src" || arg == "-S":
			if i+1 >= len(args) {
				return opts, nil, fmt.Errorf("%s requires a value", arg)
			}
			opts.ChromiumSrc = strings.TrimSpace(args[i+1])
			i += 2
		case strings.HasPrefix(arg, "--chromium-src="):
			opts.ChromiumSrc = strings.TrimSpace(strings.TrimPrefix(arg, "--chromium-src="))
			i++
		case arg == "--verbose" || arg == "-v":
			opts.Verbose = true
			i++
		case arg == "--quiet" || arg == "-q":
			opts.Quiet = true
			i++
		default:
			return opts, nil, fmt.Errorf("unknown dev flag %q", arg)
		}
	}

	return opts, args[i:], nil
}

func parseAddUpdateFlags(args []string) (addUpdateFlags, error) {
	var out addUpdateFlags

	i := 0
	for i < len(args) {
		arg := args[i]
		switch {
		case arg == "--name" || arg == "-n":
			if i+1 >= len(args) {
				return out, fmt.Errorf("%s requires a value", arg)
			}
			out.Name = strings.TrimSpace(args[i+1])
			i += 2
		case strings.HasPrefix(arg, "--name="):
			out.Name = strings.TrimSpace(strings.TrimPrefix(arg, "--name="))
			i++
		case arg == "--commit" || arg == "-c":
			if i+1 >= len(args) {
				return out, fmt.Errorf("%s requires a value", arg)
			}
			out.Commit = strings.TrimSpace(args[i+1])
			i += 2
		case strings.HasPrefix(arg, "--commit="):
			out.Commit = strings.TrimSpace(strings.TrimPrefix(arg, "--commit="))
			i++
		case arg == "--description" || arg == "-d":
			if i+1 >= len(args) {
				return out, fmt.Errorf("%s requires a value", arg)
			}
			out.Description = strings.TrimSpace(args[i+1])
			i += 2
		case strings.HasPrefix(arg, "--description="):
			out.Description = strings.TrimSpace(strings.TrimPrefix(arg, "--description="))
			i++
		default:
			return out, fmt.Errorf("unknown add-update argument %q", arg)
		}
	}

	if out.Name == "" {
		return out, fmt.Errorf("missing required --name")
	}
	if out.Commit == "" {
		return out, fmt.Errorf("missing required --commit")
	}
	if out.Description == "" {
		return out, fmt.Errorf("missing required --description")
	}

	return out, nil
}
