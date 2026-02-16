package ota

import "path/filepath"

const (
	sparkleNS = "http://www.andymatuschak.org/xml-namespaces/sparkle"
)

type ServerPlatform struct {
	Name   string
	Binary string
	OS     string
	Arch   string
}

var serverPlatforms = []ServerPlatform{
	{Name: "darwin_arm64", Binary: "browseros-server-darwin-arm64", OS: "macos", Arch: "arm64"},
	{Name: "darwin_x64", Binary: "browseros-server-darwin-x64", OS: "macos", Arch: "x86_64"},
	{Name: "linux_arm64", Binary: "browseros-server-linux-arm64", OS: "linux", Arch: "arm64"},
	{Name: "linux_x64", Binary: "browseros-server-linux-x64", OS: "linux", Arch: "x86_64"},
	{Name: "windows_x64", Binary: "browseros-server-windows-x64.exe", OS: "windows", Arch: "x86_64"},
}

type SignedArtifact struct {
	Platform  string
	ZipPath   string
	Signature string
	Length    int64
	OS        string
	Arch      string
}

type ExistingAppcast struct {
	Version   string
	PubDate   string
	Artifacts map[string]SignedArtifact
}

func appcastPath(packagesDir string, channel string) string {
	if channel == "alpha" {
		return filepath.Join(packagesDir, "build", "config", "appcast", "appcast-server.alpha.xml")
	}
	return filepath.Join(packagesDir, "build", "config", "appcast", "appcast-server.xml")
}

func platformByName(name string) (ServerPlatform, bool) {
	for _, platform := range serverPlatforms {
		if platform.Name == name {
			return platform, true
		}
	}
	return ServerPlatform{}, false
}
