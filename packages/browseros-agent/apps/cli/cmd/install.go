package cmd

import (
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"time"

	"browseros-cli/output"

	"github.com/fatih/color"
	"github.com/spf13/cobra"
)

func init() {
	cmd := &cobra.Command{
		Use:   "install",
		Short: "Download and install BrowserOS for the current platform",
		Long: `Download BrowserOS for your platform and start the installation.

macOS:   Downloads .dmg, mounts it, and copies BrowserOS to /Applications
Windows: Downloads installer .exe and launches it
Linux:   Downloads AppImage (or .deb with --deb flag)

After installation:
  browseros-cli launch        # start BrowserOS
  browseros-cli init --auto   # configure the CLI`,
		Annotations: map[string]string{"group": "Setup:"},
		Args:        cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			dir, _ := cmd.Flags().GetString("dir")
			deb, _ := cmd.Flags().GetBool("deb")

			if deb && runtime.GOOS != "linux" {
				output.Error("--deb is only available on Linux", 1)
			}

			downloadURL, filename := resolveDownload(deb)
			destPath := filepath.Join(dir, filename)

			bold := color.New(color.Bold)
			green := color.New(color.FgGreen)
			dim := color.New(color.Faint)

			bold.Printf("Downloading BrowserOS for %s...\n", platformDisplayName())
			dim.Printf("  %s\n", downloadURL)
			fmt.Println()

			client := &http.Client{Timeout: 10 * time.Minute}
			resp, err := client.Get(downloadURL)
			if err != nil {
				output.Errorf(1, "download failed: %v", err)
			}
			defer resp.Body.Close()

			if resp.StatusCode != http.StatusOK {
				output.Errorf(1, "download failed: HTTP %d", resp.StatusCode)
			}

			file, err := os.Create(destPath)
			if err != nil {
				output.Errorf(1, "create file: %v", err)
			}

			written, err := io.Copy(file, resp.Body)
			file.Close()
			if err != nil {
				os.Remove(destPath)
				output.Errorf(1, "download interrupted: %v", err)
			}

			green.Printf("Downloaded %s (%.1f MB)\n", filename, float64(written)/(1024*1024))
			fmt.Println()

			runPostInstall(destPath, deb, dim)

			fmt.Println()
			bold.Println("Next steps:")
			dim.Println("  browseros-cli launch        # start BrowserOS")
			dim.Println("  browseros-cli init --auto   # configure the CLI")
		},
	}

	cmd.Flags().String("dir", ".", "Directory to download the installer to")
	cmd.Flags().Bool("deb", false, "Download .deb package instead of AppImage (Linux only)")

	rootCmd.AddCommand(cmd)
}

func resolveDownload(deb bool) (url, filename string) {
	switch runtime.GOOS {
	case "darwin":
		return "https://files.browseros.com/download/BrowserOS.dmg", "BrowserOS.dmg"
	case "windows":
		return "https://files.browseros.com/download/BrowserOS_installer.exe", "BrowserOS_installer.exe"
	case "linux":
		if deb {
			return "https://cdn.browseros.com/download/BrowserOS.deb", "BrowserOS.deb"
		}
		return "https://files.browseros.com/download/BrowserOS.AppImage", "BrowserOS.AppImage"
	default:
		output.Errorf(1, "unsupported platform: %s/%s\n  Download manually from https://browseros.com", runtime.GOOS, runtime.GOARCH)
		return "", ""
	}
}

func platformDisplayName() string {
	switch runtime.GOOS {
	case "darwin":
		return "macOS"
	case "windows":
		return "Windows"
	case "linux":
		return "Linux"
	default:
		return runtime.GOOS
	}
}

func runPostInstall(path string, deb bool, dim *color.Color) {
	switch runtime.GOOS {
	case "darwin":
		installMacOS(path, dim)

	case "linux":
		if deb {
			dim.Println("Install the .deb package:")
			fmt.Printf("  sudo dpkg -i %s\n", path)
		} else {
			os.Chmod(path, 0755)
			dim.Printf("AppImage is ready to run: ./%s\n", filepath.Base(path))
		}

	case "windows":
		fmt.Println("Launching installer...")
		if err := exec.Command("cmd", "/c", "start", "", path).Run(); err != nil {
			dim.Printf("Could not launch installer automatically. Run: %s\n", path)
		} else {
			dim.Println("Follow the installer prompts to complete setup.")
		}
	}
}

// installMacOS mounts the DMG and copies BrowserOS.app to /Applications.
func installMacOS(dmgPath string, dim *color.Color) {
	fmt.Println("Mounting disk image...")
	mountOut, err := exec.Command("hdiutil", "attach", dmgPath, "-nobrowse", "-quiet").Output()
	if err != nil {
		dim.Println("Could not mount DMG automatically.")
		dim.Printf("  Open it manually: open %s\n", dmgPath)
		return
	}

	// Find the mount point (last field of last line of hdiutil output)
	mountPoint := ""
	for _, line := range splitLines(string(mountOut)) {
		fields := splitTabs(line)
		if len(fields) > 0 {
			mountPoint = fields[len(fields)-1]
		}
	}

	if mountPoint == "" {
		dim.Println("DMG mounted but could not determine mount point.")
		dim.Printf("  Open it manually: open %s\n", dmgPath)
		return
	}

	// Look for BrowserOS.app in the mounted volume
	appSrc := filepath.Join(mountPoint, "BrowserOS.app")
	if _, err := os.Stat(appSrc); err != nil {
		dim.Printf("DMG mounted at %s but BrowserOS.app not found inside.\n", mountPoint)
		dim.Printf("  Check the volume manually: open %s\n", mountPoint)
		exec.Command("hdiutil", "detach", mountPoint, "-quiet").Run()
		return
	}

	appDest := "/Applications/BrowserOS.app"
	fmt.Printf("Installing to %s...\n", appDest)

	// Remove existing installation if present
	os.RemoveAll(appDest)

	// Copy using cp -R (preserves code signatures, symlinks, etc.)
	if err := exec.Command("cp", "-R", appSrc, appDest).Run(); err != nil {
		dim.Printf("Could not copy to /Applications (may need sudo).\n")
		dim.Printf("  Try: sudo cp -R \"%s\" /Applications/\n", appSrc)
		exec.Command("hdiutil", "detach", mountPoint, "-quiet").Run()
		return
	}

	// Unmount
	exec.Command("hdiutil", "detach", mountPoint, "-quiet").Run()

	// Clean up DMG
	os.Remove(dmgPath)

	fmt.Println("BrowserOS installed to /Applications/BrowserOS.app")
}

func splitLines(s string) []string {
	var lines []string
	for _, line := range filepath.SplitList(s) {
		lines = append(lines, line)
	}
	// filepath.SplitList uses : on unix, not newlines — use manual split
	result := []string{}
	start := 0
	for i := 0; i < len(s); i++ {
		if s[i] == '\n' {
			line := s[start:i]
			if len(line) > 0 {
				result = append(result, line)
			}
			start = i + 1
		}
	}
	if start < len(s) {
		result = append(result, s[start:])
	}
	return result
}

func splitTabs(s string) []string {
	result := []string{}
	start := 0
	for i := 0; i < len(s); i++ {
		if s[i] == '\t' {
			field := s[start:i]
			if len(field) > 0 {
				result = append(result, field)
			}
			start = i + 1
		}
	}
	if start < len(s) {
		field := s[start:]
		if len(field) > 0 {
			result = append(result, field)
		}
	}
	return result
}
