package ui

import (
	"encoding/json"
	"fmt"
	"strings"

	"bros/internal/patch"
)

func RenderConflictReport(conflicts []patch.ConflictInfo) string {
	if len(conflicts) == 0 {
		return ""
	}

	var b strings.Builder

	b.WriteString("\n")
	b.WriteString(ErrorStyle.Render("=== CONFLICT REPORT ==="))
	b.WriteString("\n\n")

	for i, c := range conflicts {
		b.WriteString(fmt.Sprintf("file: %s\n", c.File))
		b.WriteString(fmt.Sprintf("reject_file: %s\n", c.RejectFile))
		b.WriteString(fmt.Sprintf("patch_file: %s\n", c.PatchFile))
		if c.HunksTotal > 0 {
			b.WriteString(fmt.Sprintf("hunks_total: %d\n", c.HunksTotal))
			b.WriteString(fmt.Sprintf("hunks_failed: %d\n", c.HunksFailed))
		}
		if i < len(conflicts)-1 {
			b.WriteString("---\n")
		}
	}

	return ConflictBox.Render(b.String())
}

func RenderConflictReportJSON(conflicts []patch.ConflictInfo) string {
	data, _ := json.MarshalIndent(conflicts, "", "  ")
	return string(data)
}
