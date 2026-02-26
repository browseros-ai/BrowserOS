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
	for _, f := range r.Reset {
		b.WriteString(fmt.Sprintf("  %s %s %s\n", ModifiedPrefix, f, MutedStyle.Render("(reset to BASE)")))
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

	total := len(r.Applied) + len(r.Reset) + len(r.Conflicts) + len(r.Skipped) + len(r.Deleted)
	summary := fmt.Sprintf("Pulled %d patches", total)
	b.WriteString(SuccessStyle.Render(summary))
	b.WriteString(MutedStyle.Render(fmt.Sprintf(" (%d applied, %d reset, %d deleted, %d conflicts, %d skipped)",
		len(r.Applied), len(r.Reset), len(r.Deleted), len(r.Conflicts), len(r.Skipped))))
	b.WriteString("\n")

	return b.String()
}
