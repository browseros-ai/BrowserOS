package ota

import (
	"context"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"bros/internal/native/common"
)

type publishAppcastOptions struct {
	Channel     string
	AppcastFile string
}

func runServerPublishAppcast(args []string, packagesDir string) error {
	opts, err := parsePublishAppcastOptions(args)
	if err != nil {
		return err
	}

	sourcePath := strings.TrimSpace(opts.AppcastFile)
	if sourcePath == "" {
		sourcePath = appcastPath(packagesDir, opts.Channel)
	}
	sourcePath = filepath.Clean(sourcePath)

	info, err := os.Stat(sourcePath)
	if err != nil {
		if os.IsNotExist(err) {
			if strings.TrimSpace(opts.AppcastFile) == "" {
				return fmt.Errorf("appcast file not found: %s\nrun 'bros ota server release' first to generate the appcast", sourcePath)
			}
			return fmt.Errorf("appcast file not found: %s", sourcePath)
		}
		return fmt.Errorf("checking appcast file %s: %w", sourcePath, err)
	}
	if info.IsDir() {
		return fmt.Errorf("appcast path is a directory: %s", sourcePath)
	}

	r2Key := "appcast-server.xml"
	if opts.Channel == "alpha" {
		r2Key = "appcast-server.alpha.xml"
	}

	env := common.LoadEnv(packagesDir)
	if !env.HasR2Config() {
		return fmt.Errorf("R2 configuration not set. Required env vars: R2_ACCOUNT_ID, R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY")
	}

	r2Client, err := common.NewR2Client(env)
	if err != nil {
		return err
	}

	fmt.Printf("Uploading %s to %s...\n", filepath.Base(sourcePath), r2Key)
	if err := r2Client.UploadFile(context.Background(), sourcePath, r2Key); err != nil {
		return fmt.Errorf("upload failed: %w", err)
	}

	fmt.Printf("Published: https://cdn.browseros.com/%s\n", r2Key)
	return nil
}

func parsePublishAppcastOptions(args []string) (publishAppcastOptions, error) {
	opts := publishAppcastOptions{}

	fs := flag.NewFlagSet("server-publish-appcast", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	fs.StringVar(&opts.Channel, "channel", "alpha", "release channel: alpha or prod")
	fs.StringVar(&opts.Channel, "c", "alpha", "release channel: alpha or prod")
	fs.StringVar(&opts.AppcastFile, "file", "", "custom appcast file to upload")
	fs.StringVar(&opts.AppcastFile, "f", "", "custom appcast file to upload")

	if err := fs.Parse(args); err != nil {
		return publishAppcastOptions{}, err
	}

	opts.Channel = strings.TrimSpace(opts.Channel)
	if opts.Channel == "" {
		opts.Channel = "alpha"
	}
	if opts.Channel != "alpha" && opts.Channel != "prod" {
		return publishAppcastOptions{}, fmt.Errorf("channel must be 'alpha' or 'prod'")
	}

	opts.AppcastFile = strings.TrimSpace(opts.AppcastFile)
	if fs.NArg() != 0 {
		return publishAppcastOptions{}, fmt.Errorf("unexpected positional arguments: %s", strings.Join(fs.Args(), " "))
	}

	return opts, nil
}
