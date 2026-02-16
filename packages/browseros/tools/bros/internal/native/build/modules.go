package build

import (
	"context"
	"encoding/json"
	"encoding/xml"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"runtime"
	"sort"
	"strings"
	"time"

	brosconfig "bros/internal/config"
	"bros/internal/engine"

	"github.com/aws/aws-sdk-go-v2/aws"
	awsconfig "github.com/aws/aws-sdk-go-v2/config"
	"github.com/aws/aws-sdk-go-v2/credentials"
	"github.com/aws/aws-sdk-go-v2/service/s3"
	"github.com/aws/aws-sdk-go-v2/service/s3/types"
	"gopkg.in/yaml.v3"
)

type downloadConfig struct {
	DownloadOperations []downloadOperation `yaml:"download_operations"`
}

type downloadOperation struct {
	Name        string   `yaml:"name"`
	R2Key       string   `yaml:"r2_key"`
	Destination string   `yaml:"destination"`
	OS          []string `yaml:"os"`
	Arch        []string `yaml:"arch"`
	BuildType   string   `yaml:"build_type"`
	Executable  bool     `yaml:"executable"`
}

type copyConfig struct {
	CopyOperations []copyOperation `yaml:"copy_operations"`
}

type copyOperation struct {
	Name        string   `yaml:"name"`
	Source      string   `yaml:"source"`
	Destination string   `yaml:"destination"`
	Type        string   `yaml:"type"`
	BuildType   string   `yaml:"build_type"`
	OS          []string `yaml:"os"`
	Arch        []string `yaml:"arch"`
}

var brandingReplacements = [][2]string{
	{"The Chromium Authors. All rights reserved.", "The BrowserOS Authors. All rights reserved."},
	{"Google LLC. All rights reserved.", "The BrowserOS Authors. All rights reserved."},
	{"The Chromium Authors", "BrowserOS Software Inc"},
	{"Google Chrome", "BrowserOS"},
	{"Chromium", "BrowserOS"},
	{"Chrome", "BrowserOS"},
}

func runClean(ctx *Context) error {
	if !dirExists(ctx.ChromiumSrc) {
		return fmt.Errorf("chromium source not found: %s", ctx.ChromiumSrc)
	}

	outPath := filepath.Join(ctx.ChromiumSrc, ctx.OutDir)
	if dirExists(outPath) {
		if err := os.RemoveAll(outPath); err != nil {
			return fmt.Errorf("cleaning out dir: %w", err)
		}
	}

	if err := runCmd(ctx.ChromiumSrc, "git", "reset", "--hard", "HEAD"); err != nil {
		return err
	}
	if err := runCmd(
		ctx.ChromiumSrc, "git", "clean", "-fdx", "chrome/", "components/",
		"--exclude=third_party/", "--exclude=build_tools/", "--exclude=uc_staging/",
		"--exclude=buildtools/", "--exclude=tools/", "--exclude=build/",
	); err != nil {
		return err
	}

	sparkleDir := ctx.SparkleDir()
	if dirExists(sparkleDir) {
		if err := os.RemoveAll(sparkleDir); err != nil {
			return fmt.Errorf("cleaning sparkle dir: %w", err)
		}
	}

	return nil
}

func runGitSetup(ctx *Context) error {
	if !dirExists(ctx.ChromiumSrc) {
		return fmt.Errorf("chromium source not found: %s", ctx.ChromiumSrc)
	}
	if strings.TrimSpace(ctx.ChromiumVersion) == "" {
		return fmt.Errorf("chromium version is not set")
	}

	if err := runCmd(ctx.ChromiumSrc, "git", "fetch", "--tags", "--force"); err != nil {
		return err
	}

	tagExists, err := gitTagExists(ctx.ChromiumSrc, ctx.ChromiumVersion)
	if err != nil {
		return err
	}
	if !tagExists {
		return fmt.Errorf("git tag %s not found", ctx.ChromiumVersion)
	}

	if err := runCmd(ctx.ChromiumSrc, "git", "checkout", "tags/"+ctx.ChromiumVersion); err != nil {
		return err
	}

	gclient := "gclient"
	if runtime.GOOS == "windows" {
		gclient = "gclient.bat"
	}
	return runCmd(ctx.ChromiumSrc, gclient, "sync", "-D", "--no-history", "--shallow")
}

func runSparkleSetup(ctx *Context) error {
	if runtime.GOOS != "darwin" {
		return fmt.Errorf("sparkle_setup requires macOS")
	}

	sparkleDir := ctx.SparkleDir()
	_ = os.RemoveAll(sparkleDir)
	if err := os.MkdirAll(sparkleDir, 0o755); err != nil {
		return fmt.Errorf("creating sparkle dir: %w", err)
	}

	archivePath := filepath.Join(sparkleDir, "sparkle.tar.xz")
	if err := downloadFile(ctx.SparkleURL(), archivePath); err != nil {
		return fmt.Errorf("downloading sparkle archive: %w", err)
	}
	defer os.Remove(archivePath)

	return runCmd("", "tar", "-xJf", archivePath, "-C", sparkleDir)
}

func runConfigure(ctx *Context) error {
	if !dirExists(ctx.ChromiumSrc) {
		return fmt.Errorf("chromium source not found: %s", ctx.ChromiumSrc)
	}

	flagsPath := ctx.GNFlagsPath()
	data, err := os.ReadFile(flagsPath)
	if err != nil {
		return fmt.Errorf("reading GN flags file %s: %w", flagsPath, err)
	}

	outPath := filepath.Join(ctx.ChromiumSrc, ctx.OutDir)
	if err := os.MkdirAll(outPath, 0o755); err != nil {
		return fmt.Errorf("creating out dir: %w", err)
	}

	argsPath := ctx.GNArgsPath()
	argsContent := string(data) + fmt.Sprintf("\ntarget_cpu = %q\n", ctx.Architecture)
	if err := os.WriteFile(argsPath, []byte(argsContent), 0o644); err != nil {
		return fmt.Errorf("writing args.gn: %w", err)
	}

	gn := "gn"
	if runtime.GOOS == "windows" {
		gn = "gn.bat"
	}
	return runCmd(ctx.ChromiumSrc, gn, "gen", ctx.OutDir, "--fail-on-unused-args")
}

func runDownloadResources(ctx *Context) error {
	cfg, err := loadDownloadConfig(ctx.DownloadResourcesConfig())
	if err != nil {
		return err
	}
	filtered := filterDownloadOps(cfg.DownloadOperations, ctx)
	if len(filtered) == 0 {
		return nil
	}

	r2cfg, err := loadR2Config()
	if err != nil {
		return err
	}
	client, err := newR2Client(r2cfg)
	if err != nil {
		return err
	}

	for _, op := range filtered {
		dest := filepath.Join(ctx.PackagesDir, op.Destination)
		_ = os.Remove(dest)
		if err := downloadR2Object(client, r2cfg.Bucket, op.R2Key, dest); err != nil {
			return fmt.Errorf("%s: %w", op.Name, err)
		}
		if op.Executable {
			if err := os.Chmod(dest, 0o755); err != nil {
				return fmt.Errorf("setting executable bit on %s: %w", dest, err)
			}
		}
	}

	return nil
}

func runResources(ctx *Context) error {
	data, err := os.ReadFile(ctx.CopyResourcesConfig())
	if err != nil {
		return fmt.Errorf("reading copy_resources config: %w", err)
	}
	var cfg copyConfig
	if err := yaml.Unmarshal(data, &cfg); err != nil {
		return fmt.Errorf("parsing copy_resources config: %w", err)
	}

	for _, op := range cfg.CopyOperations {
		if !copyOpMatches(op, ctx) {
			continue
		}

		src := filepath.Join(ctx.PackagesDir, op.Source)
		dst := filepath.Join(ctx.ChromiumSrc, op.Destination)
		switch op.Type {
		case "directory":
			if !dirExists(src) {
				continue
			}
			if err := copyDir(src, dst); err != nil {
				return fmt.Errorf("%s: %w", op.Name, err)
			}
		case "files":
			matches, err := filepath.Glob(src)
			if err != nil {
				return fmt.Errorf("%s: invalid glob %s: %w", op.Name, src, err)
			}
			if err := os.MkdirAll(dst, 0o755); err != nil {
				return fmt.Errorf("%s: creating destination dir: %w", op.Name, err)
			}
			for _, match := range matches {
				info, err := os.Stat(match)
				if err != nil || info.IsDir() {
					continue
				}
				if err := copyFile(match, filepath.Join(dst, filepath.Base(match))); err != nil {
					return fmt.Errorf("%s: %w", op.Name, err)
				}
			}
		case "file":
			if !fileExists(src) {
				continue
			}
			if err := os.MkdirAll(filepath.Dir(dst), 0o755); err != nil {
				return fmt.Errorf("%s: creating destination parent: %w", op.Name, err)
			}
			if err := copyFile(src, dst); err != nil {
				return fmt.Errorf("%s: %w", op.Name, err)
			}
		default:
			return fmt.Errorf("%s: unsupported copy operation type %q", op.Name, op.Type)
		}
	}

	return nil
}

func runBundledExtensions(ctx *Context) error {
	manifestURL := "https://cdn.browseros.com/extensions/update-manifest.xml"
	manifest, err := fetchExtensionManifest(manifestURL)
	if err != nil {
		return err
	}
	if len(manifest) == 0 {
		return fmt.Errorf("no extensions found in %s", manifestURL)
	}

	outputDir := filepath.Join(ctx.ChromiumSrc, "chrome", "browser", "browseros", "bundled_extensions")
	if err := os.MkdirAll(outputDir, 0o755); err != nil {
		return fmt.Errorf("creating bundled_extensions dir: %w", err)
	}

	jsonOutput := map[string]map[string]string{}
	for _, ext := range manifest {
		dest := filepath.Join(outputDir, ext.ID+".crx")
		if err := downloadFile(ext.Codebase, dest); err != nil {
			return fmt.Errorf("downloading extension %s: %w", ext.ID, err)
		}
		jsonOutput[ext.ID] = map[string]string{
			"external_crx":     ext.ID + ".crx",
			"external_version": ext.Version,
		}
	}

	jsonPath := filepath.Join(outputDir, "bundled_extensions.json")
	data, err := json.MarshalIndent(jsonOutput, "", "  ")
	if err != nil {
		return fmt.Errorf("encoding bundled_extensions.json: %w", err)
	}
	data = append(data, '\n')
	return os.WriteFile(jsonPath, data, 0o644)
}

func runChromiumReplace(ctx *Context) error {
	replacementDir := ctx.ChromiumFilesDir()
	if !dirExists(replacementDir) {
		return nil
	}

	return filepath.WalkDir(replacementDir, func(path string, d os.DirEntry, err error) error {
		if err != nil || d.IsDir() {
			return err
		}

		rel, err := filepath.Rel(replacementDir, path)
		if err != nil {
			return err
		}
		rel = filepath.ToSlash(rel)

		ext := filepath.Ext(rel)
		if ext == ".debug" || ext == ".release" {
			if (ctx.BuildType == "debug" && ext != ".debug") || (ctx.BuildType == "release" && ext != ".release") {
				return nil
			}
			rel = strings.TrimSuffix(rel, ext)
		} else {
			base := strings.TrimSuffix(rel, filepath.Ext(rel))
			if ctx.BuildType == "debug" && fileExists(filepath.Join(replacementDir, base+filepath.Ext(rel)+".debug")) {
				return nil
			}
			if ctx.BuildType == "release" && fileExists(filepath.Join(replacementDir, base+filepath.Ext(rel)+".release")) {
				return nil
			}
		}

		dest := filepath.Join(ctx.ChromiumSrc, filepath.FromSlash(rel))
		if !fileExists(dest) {
			return fmt.Errorf("destination file not found in chromium source: %s", rel)
		}
		return copyFile(path, dest)
	})
}

func runStringReplaces(ctx *Context) error {
	targets := []string{
		filepath.Join(ctx.ChromiumSrc, "chrome", "app", "chromium_strings.grd"),
		filepath.Join(ctx.ChromiumSrc, "chrome", "app", "settings_chromium_strings.grdp"),
	}

	for _, filePath := range targets {
		if !fileExists(filePath) {
			continue
		}
		data, err := os.ReadFile(filePath)
		if err != nil {
			return fmt.Errorf("reading %s: %w", filePath, err)
		}
		content := string(data)

		for _, repl := range brandingReplacements {
			content = strings.ReplaceAll(content, repl[0], repl[1])
		}
		content = replaceGoogleNotPlay(content)

		if err := os.WriteFile(filePath, []byte(content), 0o644); err != nil {
			return fmt.Errorf("writing %s: %w", filePath, err)
		}
	}
	return nil
}

func runPatches(ctx *Context) error {
	baseCommit, err := brosconfig.ReadBaseCommit(ctx.PackagesDir)
	if err != nil {
		return err
	}
	pctx := &brosconfig.Context{
		Config:      &brosconfig.Config{Name: "build", PatchesRepo: ctx.PackagesDir},
		State:       &brosconfig.State{},
		ChromiumDir: ctx.ChromiumSrc,
		PatchesRepo: ctx.PackagesDir,
		PatchesDir:  ctx.PatchesDir(),
		BaseCommit:  baseCommit,
	}
	result, err := engine.Clone(pctx, engine.CloneOpts{
		VerifyBase: false,
		Clean:      false,
		DryRun:     false,
	})
	if err != nil {
		return err
	}
	if len(result.Conflicts) > 0 {
		return fmt.Errorf("%d patch conflicts", len(result.Conflicts))
	}
	return nil
}

func runSeriesPatches(ctx *Context) error {
	seriesDir := ctx.SeriesPatchesDir()
	if !dirExists(seriesDir) {
		return fmt.Errorf("series_patches directory not found: %s", seriesDir)
	}

	seriesFiles := []string{
		filepath.Join(seriesDir, "series"),
		filepath.Join(seriesDir, "series."+currentPlatformName()),
	}

	var patchPaths []string
	for _, seriesFile := range seriesFiles {
		if !fileExists(seriesFile) {
			continue
		}
		lines, err := os.ReadFile(seriesFile)
		if err != nil {
			return fmt.Errorf("reading %s: %w", seriesFile, err)
		}
		for _, line := range strings.Split(string(lines), "\n") {
			line = strings.TrimSpace(line)
			if line == "" || strings.HasPrefix(line, "#") {
				continue
			}
			if strings.Contains(line, " #") {
				line = strings.TrimSpace(strings.Split(line, " #")[0])
			}
			if line == "" {
				continue
			}
			patchPaths = append(patchPaths, filepath.Join(seriesDir, filepath.FromSlash(line)))
		}
	}

	for _, patchPath := range patchPaths {
		if !fileExists(patchPath) {
			return fmt.Errorf("series patch not found: %s", patchPath)
		}
		err := runCmd(ctx.ChromiumSrc, "git", "apply", "--ignore-whitespace", "--whitespace=nowarn", "-p1", patchPath)
		if err != nil {
			err = runCmd(ctx.ChromiumSrc, "git", "apply", "--3way", "--ignore-whitespace", "--whitespace=nowarn", "-p1", patchPath)
		}
		if err != nil {
			return fmt.Errorf("failed applying series patch %s: %w", patchPath, err)
		}
	}
	return nil
}

func runCompile(ctx *Context) error {
	if !fileExists(ctx.GNArgsPath()) {
		return fmt.Errorf("build not configured: missing %s", ctx.GNArgsPath())
	}
	if strings.TrimSpace(ctx.BrowserOSChromiumVersion) == "" {
		return fmt.Errorf("browseros chromium version is not set")
	}

	if err := writeChromeVersion(ctx); err != nil {
		return err
	}

	autoninja := "autoninja"
	if runtime.GOOS == "windows" {
		autoninja = "autoninja.bat"
	}
	if err := runCmd(ctx.ChromiumSrc, autoninja, "-C", ctx.OutDir, "chrome", "chromedriver"); err != nil {
		return err
	}

	chromiumApp := ctx.ChromiumAppPath()
	browserOSApp := ctx.BrowserOSAppPath()
	if fileExists(chromiumApp) && !fileExists(browserOSApp) {
		if err := os.Rename(chromiumApp, browserOSApp); err != nil {
			return fmt.Errorf("renaming %s to %s: %w", chromiumApp, browserOSApp, err)
		}
	}
	return nil
}

func runSignLinux(ctx *Context) error {
	_ = ctx
	return nil
}

func runUpload(ctx *Context) error {
	artifacts, platform := detectArtifacts(ctx)
	if len(artifacts) == 0 {
		return nil
	}

	r2cfg, err := loadR2Config()
	if err != nil {
		return err
	}
	client, err := newR2Client(r2cfg)
	if err != nil {
		return err
	}

	releasePath := ctx.ReleasePath(platform)
	for _, artifact := range artifacts {
		key := releasePath + filepath.Base(artifact.Path)
		if err := uploadR2Object(client, r2cfg.Bucket, key, artifact.Path); err != nil {
			return fmt.Errorf("uploading %s: %w", artifact.Path, err)
		}
	}

	releaseJSON := buildReleaseJSON(ctx, platform, artifacts, r2cfg.CDNBaseURL)
	jsonData, err := json.MarshalIndent(releaseJSON, "", "  ")
	if err != nil {
		return fmt.Errorf("encoding release.json: %w", err)
	}

	distDir := ctx.DistDir()
	if err := os.MkdirAll(distDir, 0o755); err != nil {
		return fmt.Errorf("creating dist dir: %w", err)
	}
	localReleaseJSON := filepath.Join(distDir, "release.json")
	if err := os.WriteFile(localReleaseJSON, append(jsonData, '\n'), 0o644); err != nil {
		return fmt.Errorf("writing local release.json: %w", err)
	}
	if err := uploadR2Object(client, r2cfg.Bucket, releasePath+"release.json", localReleaseJSON); err != nil {
		return fmt.Errorf("uploading release.json: %w", err)
	}

	return nil
}

type r2Config struct {
	AccountID       string
	AccessKeyID     string
	SecretAccessKey string
	Bucket          string
	CDNBaseURL      string
	Endpoint        string
}

func loadR2Config() (*r2Config, error) {
	accountID := strings.TrimSpace(os.Getenv("R2_ACCOUNT_ID"))
	access := strings.TrimSpace(os.Getenv("R2_ACCESS_KEY_ID"))
	secret := strings.TrimSpace(os.Getenv("R2_SECRET_ACCESS_KEY"))
	if accountID == "" || access == "" || secret == "" {
		return nil, fmt.Errorf("R2 configuration not set. Required env vars: R2_ACCOUNT_ID, R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY")
	}
	bucket := strings.TrimSpace(os.Getenv("R2_BUCKET"))
	if bucket == "" {
		bucket = "browseros"
	}
	cdnBase := strings.TrimSpace(os.Getenv("R2_CDN_BASE_URL"))
	if cdnBase == "" {
		cdnBase = "http://cdn.browseros.com"
	}
	return &r2Config{
		AccountID:       accountID,
		AccessKeyID:     access,
		SecretAccessKey: secret,
		Bucket:          bucket,
		CDNBaseURL:      strings.TrimRight(cdnBase, "/"),
		Endpoint:        fmt.Sprintf("https://%s.r2.cloudflarestorage.com", accountID),
	}, nil
}

func newR2Client(cfg *r2Config) (*s3.Client, error) {
	awsCfg, err := awsconfig.LoadDefaultConfig(
		context.Background(),
		awsconfig.WithRegion("auto"),
		awsconfig.WithCredentialsProvider(credentials.NewStaticCredentialsProvider(cfg.AccessKeyID, cfg.SecretAccessKey, "")),
	)
	if err != nil {
		return nil, fmt.Errorf("loading AWS config: %w", err)
	}
	return s3.NewFromConfig(awsCfg, func(o *s3.Options) {
		o.BaseEndpoint = aws.String(cfg.Endpoint)
		o.UsePathStyle = true
	}), nil
}

func uploadR2Object(client *s3.Client, bucket, key, localPath string) error {
	f, err := os.Open(localPath)
	if err != nil {
		return err
	}
	defer f.Close()

	_, err = client.PutObject(context.Background(), &s3.PutObjectInput{
		Bucket: aws.String(bucket),
		Key:    aws.String(key),
		Body:   f,
	})
	return err
}

func downloadR2Object(client *s3.Client, bucket, key, localPath string) error {
	out, err := client.GetObject(context.Background(), &s3.GetObjectInput{
		Bucket: aws.String(bucket),
		Key:    aws.String(key),
	})
	if err != nil {
		return err
	}
	defer out.Body.Close()

	if err := os.MkdirAll(filepath.Dir(localPath), 0o755); err != nil {
		return err
	}
	tmpPath := localPath + ".tmp"
	f, err := os.Create(tmpPath)
	if err != nil {
		return err
	}
	if _, err := io.Copy(f, out.Body); err != nil {
		f.Close()
		_ = os.Remove(tmpPath)
		return err
	}
	if err := f.Close(); err != nil {
		_ = os.Remove(tmpPath)
		return err
	}
	if err := os.Rename(tmpPath, localPath); err != nil {
		_ = os.Remove(tmpPath)
		return err
	}
	return nil
}

func loadDownloadConfig(path string) (*downloadConfig, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("reading download config %s: %w", path, err)
	}
	var cfg downloadConfig
	if err := yaml.Unmarshal(data, &cfg); err != nil {
		return nil, fmt.Errorf("parsing download config %s: %w", path, err)
	}
	return &cfg, nil
}

func filterDownloadOps(ops []downloadOperation, ctx *Context) []downloadOperation {
	currentOS := currentPlatformName()
	targetArchs := []string{ctx.Architecture}
	if ctx.Architecture == "universal" {
		targetArchs = []string{"arm64", "x64", "universal"}
	}
	out := make([]downloadOperation, 0, len(ops))
	for _, op := range ops {
		if len(op.OS) > 0 && !containsString(op.OS, currentOS) {
			continue
		}
		if len(op.Arch) > 0 {
			matched := false
			for _, arch := range targetArchs {
				if containsString(op.Arch, arch) {
					matched = true
					break
				}
			}
			if !matched {
				continue
			}
		}
		if strings.TrimSpace(op.BuildType) != "" && strings.TrimSpace(op.BuildType) != ctx.BuildType {
			continue
		}
		out = append(out, op)
	}
	return out
}

func copyOpMatches(op copyOperation, ctx *Context) bool {
	if strings.TrimSpace(op.BuildType) != "" && strings.TrimSpace(op.BuildType) != ctx.BuildType {
		return false
	}
	if len(op.OS) > 0 && !containsString(op.OS, currentPlatformName()) {
		return false
	}
	if len(op.Arch) > 0 && !containsString(op.Arch, ctx.Architecture) {
		return false
	}
	return true
}

func copyFile(src, dst string) error {
	in, err := os.Open(src)
	if err != nil {
		return err
	}
	defer in.Close()

	if err := os.MkdirAll(filepath.Dir(dst), 0o755); err != nil {
		return err
	}

	tmp := dst + ".tmp"
	out, err := os.Create(tmp)
	if err != nil {
		return err
	}
	if _, err := io.Copy(out, in); err != nil {
		out.Close()
		_ = os.Remove(tmp)
		return err
	}
	if err := out.Close(); err != nil {
		_ = os.Remove(tmp)
		return err
	}
	if err := os.Rename(tmp, dst); err != nil {
		_ = os.Remove(tmp)
		return err
	}
	return nil
}

func copyDir(src, dst string) error {
	return filepath.WalkDir(src, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			return err
		}
		rel, err := filepath.Rel(src, path)
		if err != nil {
			return err
		}
		target := filepath.Join(dst, rel)
		if d.IsDir() {
			return os.MkdirAll(target, 0o755)
		}
		return copyFile(path, target)
	})
}

func runCmd(dir string, name string, args ...string) error {
	cmd := exec.Command(name, args...)
	if strings.TrimSpace(dir) != "" {
		cmd.Dir = dir
	}
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	cmd.Stdin = os.Stdin
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("%s %s: %w", name, strings.Join(args, " "), err)
	}
	return nil
}

func gitTagExists(repoDir, tag string) (bool, error) {
	out, err := exec.Command("git", "-C", repoDir, "tag", "-l", tag).CombinedOutput()
	if err != nil {
		return false, fmt.Errorf("checking tag %s: %w (%s)", tag, err, strings.TrimSpace(string(out)))
	}
	for _, line := range strings.Split(strings.TrimSpace(string(out)), "\n") {
		if strings.TrimSpace(line) == tag {
			return true, nil
		}
	}
	return false, nil
}

func writeChromeVersion(ctx *Context) error {
	parts := strings.Split(ctx.BrowserOSChromiumVersion, ".")
	if len(parts) != 4 {
		return fmt.Errorf("invalid browseros chromium version %q", ctx.BrowserOSChromiumVersion)
	}
	content := fmt.Sprintf("MAJOR=%s\nMINOR=%s\nBUILD=%s\nPATCH=%s", parts[0], parts[1], parts[2], parts[3])
	path := filepath.Join(ctx.ChromiumSrc, "chrome", "VERSION")
	return os.WriteFile(path, []byte(content), 0o644)
}

func replaceGoogleNotPlay(content string) string {
	if !strings.Contains(content, "Google") {
		return content
	}
	var b strings.Builder
	for i := 0; i < len(content); {
		if strings.HasPrefix(content[i:], "Google") {
			if strings.HasPrefix(content[i+len("Google"):], " Play") {
				b.WriteString("Google")
			} else {
				b.WriteString("BrowserOS")
			}
			i += len("Google")
			continue
		}
		b.WriteByte(content[i])
		i++
	}
	return b.String()
}

type manifestDoc struct {
	Apps []manifestApp `xml:"app"`
}

type manifestApp struct {
	AppID  string         `xml:"appid,attr"`
	Update manifestUpdate `xml:"updatecheck"`
}

type manifestUpdate struct {
	Version  string `xml:"version,attr"`
	Codebase string `xml:"codebase,attr"`
}

type extensionInfo struct {
	ID       string
	Version  string
	Codebase string
}

func fetchExtensionManifest(url string) ([]extensionInfo, error) {
	resp, err := http.Get(url)
	if err != nil {
		return nil, fmt.Errorf("fetching extensions manifest: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return nil, fmt.Errorf("fetching extensions manifest: HTTP %d", resp.StatusCode)
	}
	data, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("reading extensions manifest: %w", err)
	}

	// Namespace-safe fallback: decode by scanning app and updatecheck tags.
	var decoded struct {
		Apps []manifestApp `xml:"app"`
	}
	if err := xml.Unmarshal(data, &decoded); err == nil && len(decoded.Apps) > 0 {
		out := make([]extensionInfo, 0, len(decoded.Apps))
		for _, app := range decoded.Apps {
			if strings.TrimSpace(app.AppID) == "" || strings.TrimSpace(app.Update.Codebase) == "" || strings.TrimSpace(app.Update.Version) == "" {
				continue
			}
			out = append(out, extensionInfo{ID: app.AppID, Version: app.Update.Version, Codebase: app.Update.Codebase})
		}
		return out, nil
	}

	// Regex fallback for namespaced XML.
	re := regexp.MustCompile(`(?s)<app[^>]*appid=['"]([^'"]+)['"][^>]*>.*?<updatecheck[^>]*codebase=['"]([^'"]+)['"][^>]*version=['"]([^'"]+)['"][^>]*/?>`)
	matches := re.FindAllStringSubmatch(string(data), -1)
	out := make([]extensionInfo, 0, len(matches))
	for _, m := range matches {
		out = append(out, extensionInfo{
			ID:       strings.TrimSpace(m[1]),
			Codebase: strings.TrimSpace(m[2]),
			Version:  strings.TrimSpace(m[3]),
		})
	}
	return out, nil
}

func downloadFile(url, dest string) error {
	resp, err := http.Get(url)
	if err != nil {
		return fmt.Errorf("GET %s: %w", url, err)
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return fmt.Errorf("GET %s: HTTP %d", url, resp.StatusCode)
	}
	if err := os.MkdirAll(filepath.Dir(dest), 0o755); err != nil {
		return err
	}
	tmp := dest + ".tmp"
	f, err := os.Create(tmp)
	if err != nil {
		return err
	}
	if _, err := io.Copy(f, resp.Body); err != nil {
		f.Close()
		_ = os.Remove(tmp)
		return err
	}
	if err := f.Close(); err != nil {
		_ = os.Remove(tmp)
		return err
	}
	if err := os.Rename(tmp, dest); err != nil {
		_ = os.Remove(tmp)
		return err
	}
	return nil
}

type localArtifact struct {
	Path     string
	Key      string
	Size     int64
	Metadata map[string]any
}

func detectArtifacts(ctx *Context) ([]localArtifact, string) {
	distDir := ctx.DistDir()
	if !dirExists(distDir) {
		return nil, platformForR2()
	}

	entries, err := os.ReadDir(distDir)
	if err != nil {
		return nil, platformForR2()
	}

	var artifacts []localArtifact
	for _, entry := range entries {
		if entry.IsDir() {
			continue
		}
		name := entry.Name()
		ext := strings.ToLower(filepath.Ext(name))
		keep := false
		switch platformForR2() {
		case "macos":
			keep = ext == ".dmg"
		case "win":
			keep = ext == ".exe" || ext == ".zip"
		default:
			keep = ext == ".appimage" || ext == ".deb"
		}
		if !keep {
			continue
		}
		fullPath := filepath.Join(distDir, name)
		info, err := os.Stat(fullPath)
		if err != nil {
			continue
		}
		artifacts = append(artifacts, localArtifact{
			Path:     fullPath,
			Key:      artifactKey(name, platformForR2()),
			Size:     info.Size(),
			Metadata: artifactMetadataForFile(ctx, name),
		})
	}
	sort.Slice(artifacts, func(i, j int) bool { return artifacts[i].Path < artifacts[j].Path })
	return artifacts, platformForR2()
}

func artifactKey(filename, platform string) string {
	lower := strings.ToLower(filename)
	switch platform {
	case "macos":
		switch {
		case strings.Contains(lower, "arm64"):
			return "arm64"
		case strings.Contains(lower, "x64") || strings.Contains(lower, "x86_64"):
			return "x64"
		case strings.Contains(lower, "universal"):
			return "universal"
		}
	case "win":
		switch {
		case strings.Contains(lower, "installer.exe"):
			return "x64_installer"
		case strings.Contains(lower, "installer.zip"):
			return "x64_zip"
		}
	default:
		switch {
		case strings.HasSuffix(lower, ".appimage"):
			return "x64_appimage"
		case strings.HasSuffix(lower, ".deb"):
			return "x64_deb"
		}
	}
	return strings.TrimSuffix(filename, filepath.Ext(filename))
}

func platformForR2() string {
	switch runtime.GOOS {
	case "darwin":
		return "macos"
	case "windows":
		return "win"
	default:
		return "linux"
	}
}

func buildReleaseJSON(ctx *Context, platform string, artifacts []localArtifact, cdnBase string) map[string]any {
	releasePath := strings.TrimLeft(ctx.ReleasePath(platform), "/")
	obj := map[string]any{
		"platform":                   platform,
		"version":                    ctx.SemanticVersion,
		"chromium_version":           ctx.ChromiumVersion,
		"browseros_chromium_version": ctx.BrowserOSChromiumVersion,
		"build_date":                 time.Now().UTC().Format(time.RFC3339),
		"artifacts":                  map[string]any{},
	}
	if platform == "macos" && strings.TrimSpace(ctx.SparkleVersion) != "" {
		obj["sparkle_version"] = ctx.SparkleVersion
	}
	artMap := obj["artifacts"].(map[string]any)
	for _, art := range artifacts {
		filename := filepath.Base(art.Path)
		entry := map[string]any{
			"filename": filename,
			"url":      strings.TrimRight(cdnBase, "/") + "/" + releasePath + filename,
			"size":     art.Size,
		}
		for k, v := range art.Metadata {
			entry[k] = v
		}
		artMap[art.Key] = entry
	}
	return obj
}

func artifactMetadataForFile(ctx *Context, filename string) map[string]any {
	out := map[string]any{}
	if sig, ok := ctx.SparkleSignatures[filename]; ok {
		out["sparkle_signature"] = sig.Signature
		out["sparkle_length"] = sig.Length
	}
	return out
}

func copyR2Object(client *s3.Client, bucket, sourceKey, destKey string) error {
	_, err := client.CopyObject(context.Background(), &s3.CopyObjectInput{
		Bucket:            aws.String(bucket),
		CopySource:        aws.String(bucket + "/" + sourceKey),
		Key:               aws.String(destKey),
		MetadataDirective: types.MetadataDirectiveCopy,
	})
	return err
}
