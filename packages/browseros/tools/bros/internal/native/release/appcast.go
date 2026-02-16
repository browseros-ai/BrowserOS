package release

import (
	"fmt"
	"time"
)

type AppcastSnippet struct {
	Arch     string
	Filename string
	ItemXML  string
}

func GenerateAppcastItem(artifact Artifact, version, sparkleVersion, buildDate string) string {
	signature := artifact.SparkleSignature
	length := artifact.Size
	if artifact.SparkleLength > 0 {
		length = artifact.SparkleLength
	}

	return fmt.Sprintf(`<item>
  <title>BrowserOS - %s</title>
  <description sparkle:format="plain-text">
  </description>
  <sparkle:version>%s</sparkle:version>
  <sparkle:shortVersionString>%s</sparkle:shortVersionString>
  <pubDate>%s</pubDate>
  <link>https://browseros.com</link>
  <enclosure
    url="%s"
    sparkle:edSignature="%s"
    length="%d"
    type="application/octet-stream" />
  <sparkle:minimumSystemVersion>10.15</sparkle:minimumSystemVersion>
</item>`, version, sparkleVersion, version, formatPubDate(buildDate), artifact.URL, signature, length)
}

func GenerateAppcastSnippets(version string, macOSMetadata PlatformMetadata) []AppcastSnippet {
	archToFile := map[string]string{
		"arm64":     "appcast.xml",
		"x64":       "appcast-x86_64.xml",
		"universal": "appcast.xml",
	}

	orderedArch := []string{"arm64", "x64", "universal"}
	out := make([]AppcastSnippet, 0, len(orderedArch))

	for _, arch := range orderedArch {
		artifact, ok := macOSMetadata.Artifacts[arch]
		if !ok {
			continue
		}
		out = append(out, AppcastSnippet{
			Arch:     arch,
			Filename: archToFile[arch],
			ItemXML:  GenerateAppcastItem(artifact, version, macOSMetadata.SparkleVersion, macOSMetadata.BuildDate),
		})
	}

	return out
}

func formatPubDate(buildDate string) string {
	if buildDate == "" {
		return ""
	}

	for _, layout := range []string{time.RFC3339Nano, time.RFC3339} {
		if parsed, err := time.Parse(layout, buildDate); err == nil {
			return parsed.Format("Mon, 02 Jan 2006 15:04:05 -0700")
		}
	}

	return buildDate
}
