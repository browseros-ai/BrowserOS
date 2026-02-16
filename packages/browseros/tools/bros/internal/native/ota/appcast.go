package ota

import (
	"encoding/xml"
	"fmt"
	"os"
	"path"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"
)

var platformFromFilenamePattern = regexp.MustCompile(`_([a-z]+_[a-z0-9]+)\.zip$`)

func parseExistingAppcast(appcastPath string) (*ExistingAppcast, error) {
	data, err := os.ReadFile(appcastPath)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, fmt.Errorf("reading appcast %s: %w", appcastPath, err)
	}

	var doc appcastRSS
	if err := xml.Unmarshal(data, &doc); err != nil {
		return nil, fmt.Errorf("malformed appcast XML: %w", err)
	}

	if doc.Channel == nil || doc.Channel.Item == nil {
		return nil, nil
	}

	item := doc.Channel.Item
	if strings.TrimSpace(item.Version) == "" {
		return nil, nil
	}

	artifacts := make(map[string]SignedArtifact)
	for _, enclosure := range item.Enclosures {
		if enclosure.URL == "" || enclosure.OS == "" || enclosure.Arch == "" || enclosure.Signature == "" {
			continue
		}

		filename := path.Base(enclosure.URL)
		matches := platformFromFilenamePattern.FindStringSubmatch(filename)
		if len(matches) != 2 {
			continue
		}

		length := int64(0)
		if strings.TrimSpace(enclosure.Length) != "" {
			if parsed, parseErr := strconv.ParseInt(strings.TrimSpace(enclosure.Length), 10, 64); parseErr == nil {
				length = parsed
			}
		}

		platform := matches[1]
		artifacts[platform] = SignedArtifact{
			Platform:  platform,
			ZipPath:   filename,
			Signature: enclosure.Signature,
			Length:    length,
			OS:        enclosure.OS,
			Arch:      enclosure.Arch,
		}
	}

	return &ExistingAppcast{
		Version:   strings.TrimSpace(item.Version),
		PubDate:   strings.TrimSpace(item.PubDate),
		Artifacts: artifacts,
	}, nil
}

func generateServerAppcast(version string, artifacts []SignedArtifact, channel string, existing *ExistingAppcast) string {
	title := "BrowserOS Server"
	appcastURL := "https://cdn.browseros.com/appcast-server.xml"
	if channel == "alpha" {
		title = "BrowserOS Server (Alpha)"
		appcastURL = "https://cdn.browseros.com/appcast-server.alpha.xml"
	}

	pubDate := time.Now().UTC().Format("Mon, 02 Jan 2006 15:04:05 +0000")
	finalArtifacts := make([]SignedArtifact, 0, len(artifacts))

	if existing != nil && existing.Version == version {
		merged := make(map[string]SignedArtifact, len(existing.Artifacts)+len(artifacts))
		for platform, artifact := range existing.Artifacts {
			merged[platform] = artifact
		}
		for _, artifact := range artifacts {
			merged[artifact.Platform] = artifact
		}
		for _, artifact := range merged {
			finalArtifacts = append(finalArtifacts, artifact)
		}
		if existing.PubDate != "" {
			pubDate = existing.PubDate
		}
	} else {
		finalArtifacts = append(finalArtifacts, artifacts...)
	}

	sort.Slice(finalArtifacts, func(i, j int) bool {
		return finalArtifacts[i].Platform < finalArtifacts[j].Platform
	})

	var enclosureBuilder strings.Builder
	for idx, artifact := range finalArtifacts {
		if idx > 0 {
			enclosureBuilder.WriteString("\n\n")
		}

		comment := displayOSLabel(artifact.OS) + " " + artifact.Arch

		zipFilename := fmt.Sprintf("browseros_server_%s_%s.zip", version, artifact.Platform)
		url := fmt.Sprintf("https://cdn.browseros.com/server/%s", zipFilename)

		enclosureBuilder.WriteString(fmt.Sprintf("      <!-- %s -->\n", comment))
		enclosureBuilder.WriteString("      <enclosure\n")
		enclosureBuilder.WriteString(fmt.Sprintf("        url=\"%s\"\n", xmlEscape(url)))
		enclosureBuilder.WriteString(fmt.Sprintf("        sparkle:os=\"%s\"\n", xmlEscape(artifact.OS)))
		enclosureBuilder.WriteString(fmt.Sprintf("        sparkle:arch=\"%s\"\n", xmlEscape(artifact.Arch)))
		enclosureBuilder.WriteString(fmt.Sprintf("        sparkle:edSignature=\"%s\"\n", xmlEscape(artifact.Signature)))
		enclosureBuilder.WriteString(fmt.Sprintf("        length=\"%d\"\n", artifact.Length))
		enclosureBuilder.WriteString("        type=\"application/zip\"/>")
	}

	return fmt.Sprintf(`<?xml version="1.0" encoding="utf-8"?>
<rss xmlns:sparkle="%s" version="2.0">
  <channel>
    <title>%s</title>
    <link>%s</link>
    <description>BrowserOS Server binary updates</description>
    <language>en</language>

    <item>
      <sparkle:version>%s</sparkle:version>
      <pubDate>%s</pubDate>

%s
    </item>

  </channel>
</rss>
`, sparkleNS, xmlEscape(title), xmlEscape(appcastURL), xmlEscape(version), xmlEscape(pubDate), enclosureBuilder.String())
}

type appcastRSS struct {
	XMLName xml.Name        `xml:"rss"`
	Channel *appcastChannel `xml:"channel"`
}

type appcastChannel struct {
	Item *appcastItem `xml:"item"`
}

type appcastItem struct {
	Version    string             `xml:"http://www.andymatuschak.org/xml-namespaces/sparkle version"`
	PubDate    string             `xml:"pubDate"`
	Enclosures []appcastEnclosure `xml:"enclosure"`
}

type appcastEnclosure struct {
	URL       string `xml:"url,attr"`
	OS        string `xml:"http://www.andymatuschak.org/xml-namespaces/sparkle os,attr"`
	Arch      string `xml:"http://www.andymatuschak.org/xml-namespaces/sparkle arch,attr"`
	Signature string `xml:"http://www.andymatuschak.org/xml-namespaces/sparkle edSignature,attr"`
	Length    string `xml:"length,attr"`
}

func xmlEscape(value string) string {
	var b strings.Builder
	_ = xml.EscapeText(&b, []byte(value))
	return b.String()
}

func displayOSLabel(osName string) string {
	switch osName {
	case "macos":
		return "macOS"
	case "windows":
		return "Windows"
	case "linux":
		return "Linux"
	default:
		if osName == "" {
			return "Unknown"
		}
		return osName
	}
}
