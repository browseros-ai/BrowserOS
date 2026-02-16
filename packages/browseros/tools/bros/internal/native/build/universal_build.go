package build

import (
	"bytes"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"runtime"
	"strings"
)

var universalArchitectures = []string{"arm64", "x64"}

func runUniversalBuild(ctx *Context) error {
	if runtime.GOOS != "darwin" {
		return fmt.Errorf("Universal builds only supported on macOS")
	}
	if err := validateMacOSSigningEnv(ctx); err != nil {
		return err
	}

	if err := cleanUniversalBuildDirectories(ctx); err != nil {
		return err
	}

	archApps := map[string]string{}
	for _, arch := range universalArchitectures {
		archCtx, err := newContext(ctx.PackagesDir, ctx.ChromiumSrc, arch, ctx.BuildType)
		if err != nil {
			return err
		}
		archCtx.FixedAppPath = filepath.Join(ctx.ChromiumSrc, "out", "Default_"+arch, "BrowserOS.app")

		if err := runResources(archCtx); err != nil {
			return fmt.Errorf("%s resources: %w", arch, err)
		}
		if err := runConfigure(archCtx); err != nil {
			return fmt.Errorf("%s configure: %w", arch, err)
		}
		if err := runCompile(archCtx); err != nil {
			return fmt.Errorf("%s compile: %w", arch, err)
		}
		if !dirExists(archCtx.FixedAppPath) {
			return fmt.Errorf("%s build failed, app not found: %s", arch, archCtx.FixedAppPath)
		}

		if err := runSignMacOS(archCtx); err != nil {
			return fmt.Errorf("%s sign: %w", arch, err)
		}
		if err := runPackageMacOS(archCtx); err != nil {
			return fmt.Errorf("%s package: %w", arch, err)
		}
		_ = runUpload(archCtx)

		archApps[arch] = archCtx.FixedAppPath
	}

	universalApp := filepath.Join(ctx.ChromiumSrc, "out", "Default_universal", "BrowserOS.app")
	if err := mergeMacAppBundles(archApps["arm64"], archApps["x64"], universalApp); err != nil {
		return err
	}

	universalCtx, err := newContext(ctx.PackagesDir, ctx.ChromiumSrc, "universal", ctx.BuildType)
	if err != nil {
		return err
	}
	universalCtx.OutDir = filepath.Join("out", "Default_universal")
	universalCtx.FixedAppPath = universalApp

	if err := runSignMacOS(universalCtx); err != nil {
		return fmt.Errorf("universal sign: %w", err)
	}
	if err := runPackageMacOS(universalCtx); err != nil {
		return fmt.Errorf("universal package: %w", err)
	}
	_ = runUpload(universalCtx)

	return nil
}

func cleanUniversalBuildDirectories(ctx *Context) error {
	for _, arch := range universalArchitectures {
		path := filepath.Join(ctx.ChromiumSrc, "out", "Default_"+arch)
		if dirExists(path) {
			if err := os.RemoveAll(path); err != nil {
				return fmt.Errorf("cleaning %s: %w", path, err)
			}
		}
	}
	universalPath := filepath.Join(ctx.ChromiumSrc, "out", "Default_universal")
	if dirExists(universalPath) {
		if err := os.RemoveAll(universalPath); err != nil {
			return fmt.Errorf("cleaning %s: %w", universalPath, err)
		}
	}
	return nil
}

func mergeMacAppBundles(arm64App, x64App, outputApp string) error {
	if !dirExists(arm64App) {
		return fmt.Errorf("arm64 app not found: %s", arm64App)
	}
	if !dirExists(x64App) {
		return fmt.Errorf("x64 app not found: %s", x64App)
	}

	if dirExists(outputApp) {
		if err := os.RemoveAll(outputApp); err != nil {
			return fmt.Errorf("removing existing universal app: %w", err)
		}
	}
	if err := os.MkdirAll(filepath.Dir(outputApp), 0o755); err != nil {
		return fmt.Errorf("creating universal output dir: %w", err)
	}
	if err := runCmd("", "ditto", arm64App, outputApp); err != nil {
		return fmt.Errorf("copying arm64 app bundle: %w", err)
	}

	err := filepath.WalkDir(outputApp, func(outPath string, d fs.DirEntry, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		rel, err := filepath.Rel(outputApp, outPath)
		if err != nil {
			return err
		}
		if rel == "." {
			return nil
		}

		x64Path := filepath.Join(x64App, rel)
		x64Info, err := os.Lstat(x64Path)
		if err != nil {
			if os.IsNotExist(err) {
				return nil
			}
			return err
		}

		outInfo, err := os.Lstat(outPath)
		if err != nil {
			return err
		}

		if outInfo.IsDir() || x64Info.IsDir() {
			return nil
		}

		if outInfo.Mode()&os.ModeSymlink != 0 && x64Info.Mode()&os.ModeSymlink != 0 {
			armTarget, err := os.Readlink(outPath)
			if err != nil {
				return err
			}
			x64Target, err := os.Readlink(x64Path)
			if err != nil {
				return err
			}
			if armTarget != x64Target {
				return fmt.Errorf("symlink mismatch for %s: %s vs %s", rel, armTarget, x64Target)
			}
			return nil
		}

		identical, err := filesEqual(outPath, x64Path)
		if err != nil {
			return err
		}
		if identical {
			return nil
		}

		baseName := filepath.Base(outPath)
		if isInfoPlistPath(baseName) || baseName == "CodeResources" {
			return nil
		}

		isMachOArm := isMachOFile(outPath)
		isMachOX64 := isMachOFile(x64Path)
		if isMachOArm && isMachOX64 {
			return createUniversalBinaryAtPath(outPath, x64Path)
		}

		return fmt.Errorf("non-Mach-O files differ and cannot be merged: %s", rel)
	})
	if err != nil {
		return err
	}

	return copyMissingFilesFromSecondTree(x64App, outputApp)
}

func copyMissingFilesFromSecondTree(sourceRoot, destRoot string) error {
	return filepath.WalkDir(sourceRoot, func(sourcePath string, d fs.DirEntry, walkErr error) error {
		if walkErr != nil {
			return walkErr
		}
		rel, err := filepath.Rel(sourceRoot, sourcePath)
		if err != nil {
			return err
		}
		if rel == "." {
			return nil
		}
		destPath := filepath.Join(destRoot, rel)
		if _, err := os.Lstat(destPath); err == nil {
			return nil
		} else if !os.IsNotExist(err) {
			return err
		}

		info, err := os.Lstat(sourcePath)
		if err != nil {
			return err
		}

		if info.Mode()&os.ModeSymlink != 0 {
			target, err := os.Readlink(sourcePath)
			if err != nil {
				return err
			}
			if err := os.Symlink(target, destPath); err != nil {
				return err
			}
			return nil
		}

		if d.IsDir() {
			return os.MkdirAll(destPath, info.Mode().Perm())
		}

		if err := os.MkdirAll(filepath.Dir(destPath), 0o755); err != nil {
			return err
		}
		return copyFile(sourcePath, destPath)
	})
}

func createUniversalBinaryAtPath(armFile, x64File string) error {
	temp := armFile + ".universal_tmp"
	_ = os.Remove(temp)
	if err := runCmd("", "lipo", "-create", "-output", temp, "-segalign", "x86_64", "0x4000", "-segalign", "arm64", "0x4000", armFile, x64File); err != nil {
		return fmt.Errorf("lipo merge failed for %s: %w", armFile, err)
	}
	if err := os.Rename(temp, armFile); err != nil {
		_ = os.Remove(temp)
		return err
	}
	return nil
}

func isMachOFile(path string) bool {
	result, err := runCmdCapture("", "file", "-b", path)
	if err != nil || result.ExitCode != 0 {
		return false
	}
	return strings.Contains(result.Stdout, "Mach-O")
}

func isInfoPlistPath(baseName string) bool {
	if baseName == "Info.plist" {
		return true
	}
	return strings.HasSuffix(baseName, "-Info.plist")
}

func filesEqual(pathA, pathB string) (bool, error) {
	infoA, err := os.Stat(pathA)
	if err != nil {
		return false, err
	}
	infoB, err := os.Stat(pathB)
	if err != nil {
		return false, err
	}
	if infoA.Size() != infoB.Size() {
		return false, nil
	}

	fA, err := os.Open(pathA)
	if err != nil {
		return false, err
	}
	defer fA.Close()

	fB, err := os.Open(pathB)
	if err != nil {
		return false, err
	}
	defer fB.Close()

	bufA := make([]byte, 64*1024)
	bufB := make([]byte, 64*1024)
	for {
		nA, errA := fA.Read(bufA)
		nB, errB := fB.Read(bufB)
		if nA != nB {
			return false, nil
		}
		if nA > 0 && !bytes.Equal(bufA[:nA], bufB[:nB]) {
			return false, nil
		}

		if errA == io.EOF && errB == io.EOF {
			return true, nil
		}
		if errA != nil && errA != io.EOF {
			return false, errA
		}
		if errB != nil && errB != io.EOF {
			return false, errB
		}
	}
}
