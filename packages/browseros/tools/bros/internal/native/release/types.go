package release

const (
	PlatformMacOS = "macos"
	PlatformWin   = "win"
	PlatformLinux = "linux"
)

var Platforms = []string{PlatformMacOS, PlatformWin, PlatformLinux}

var PlatformDisplayNames = map[string]string{
	PlatformMacOS: "macOS",
	PlatformWin:   "Windows",
	PlatformLinux: "Linux",
}

var DownloadPathMapping = map[string]map[string]string{
	PlatformMacOS: {
		"arm64":     "download/BrowserOS-arm64.dmg",
		"x64":       "download/BrowserOS-x86_64.dmg",
		"universal": "download/BrowserOS.dmg",
	},
	PlatformWin: {
		"x64_installer": "download/BrowserOS_installer.exe",
	},
	PlatformLinux: {
		"x64_appimage": "download/BrowserOS.AppImage",
		"x64_deb":      "download/browseros.deb",
	},
}

var OSNameMap = map[string]string{
	"macos":   PlatformMacOS,
	"mac":     PlatformMacOS,
	"windows": PlatformWin,
	"win":     PlatformWin,
	"linux":   PlatformLinux,
}

type Artifact struct {
	Filename         string `json:"filename"`
	URL              string `json:"url"`
	Size             int64  `json:"size"`
	SparkleSignature string `json:"sparkle_signature,omitempty"`
	SparkleLength    int64  `json:"sparkle_length,omitempty"`
}

type PlatformMetadata struct {
	BuildDate       string              `json:"build_date"`
	ChromiumVersion string              `json:"chromium_version"`
	SparkleVersion  string              `json:"sparkle_version,omitempty"`
	Artifacts       map[string]Artifact `json:"artifacts"`
}
