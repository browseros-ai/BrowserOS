package dev

import (
	"fmt"
	"strings"
)

// Options are top-level flags for `bros dev`.
type Options struct {
	ChromiumSrc string
	Verbose     bool
	Quiet       bool
}

// Run executes the native `dev` command family.
// rawArgs may include the leading "dev" token.
func Run(rawArgs []string) error {
	args := append([]string(nil), rawArgs...)
	if len(args) > 0 && args[0] == "dev" {
		args = args[1:]
	}

	opts, rest, err := parseGlobalFlags(args)
	if err != nil {
		return err
	}
	if len(rest) == 0 {
		return fmt.Errorf("dev command requires a subcommand")
	}

	ctx, err := newContext(opts)
	if err != nil {
		return err
	}

	switch rest[0] {
	case "status":
		if len(rest) != 1 {
			return fmt.Errorf("status does not accept arguments")
		}
		return runStatus(ctx)
	case "annotate":
		if len(rest) > 2 {
			return fmt.Errorf("annotate accepts at most one feature name")
		}
		featureFilter := ""
		if len(rest) == 2 {
			featureFilter = strings.TrimSpace(rest[1])
		}
		return runAnnotate(ctx, featureFilter)
	case "feature":
		return runFeature(ctx, rest[1:])
	default:
		return fmt.Errorf("unknown dev subcommand %q", rest[0])
	}
}

func runFeature(ctx *Context, args []string) error {
	if len(args) == 0 {
		return fmt.Errorf("feature requires a subcommand")
	}

	switch args[0] {
	case "list":
		if len(args) != 1 {
			return fmt.Errorf("feature list does not accept arguments")
		}
		return runFeatureList(ctx)
	case "show":
		if len(args) != 2 {
			return fmt.Errorf("usage: feature show <feature_name>")
		}
		return runFeatureShow(ctx, args[1])
	case "add-update":
		parsed, err := parseAddUpdateFlags(args[1:])
		if err != nil {
			return err
		}
		return runFeatureAddUpdate(ctx, parsed)
	case "classify":
		if len(args) != 1 {
			return fmt.Errorf("feature classify does not accept arguments")
		}
		return runFeatureClassify(ctx)
	default:
		return fmt.Errorf("unknown feature subcommand %q", args[0])
	}
}
