package ui

import (
	"fmt"
	"strings"

	"bros/internal/patch"
)

func RenderPullResult(r *patch.PullResult) string {
	var b strings.Builder

	b.WriteString(TitleStyle.Render("bros pull"))
	b.WriteString("\n\n")

	for _, f := range r.Applied {
		b.WriteString(fmt.Sprintf("  %s %s\n", SuccessStyle.Render("+"), f))
	}
	for _, c := range r.Conflicts {
		b.WriteString(fmt.Sprintf("  %s %s\n", ErrorStyle.Render("x"), c.File))
	}
	for _, f := range r.Deleted {
		b.WriteString(fmt.Sprintf("  %s %s\n", DeletedPrefix, f))
	}
	if len(r.Skipped) > 0 {
		b.WriteString(fmt.Sprintf("  %s %s\n", SkippedPrefix,
			MutedStyle.Render(fmt.Sprintf("%d files skipped (already up to date)", len(r.Skipped)))))
	}

	b.WriteString("\n")

	total := len(r.Applied) + len(r.Conflicts) + len(r.Skipped)
	summary := fmt.Sprintf("Pulled %d patches", total)
	b.WriteString(SuccessStyle.Render(summary))
	b.WriteString(MutedStyle.Render(fmt.Sprintf(" (%d applied, %d conflicts, %d skipped)",
		len(r.Applied), len(r.Conflicts), len(r.Skipped))))
	b.WriteString("\n")

	return b.String()
}
