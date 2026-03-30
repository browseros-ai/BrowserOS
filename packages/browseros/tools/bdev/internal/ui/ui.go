package ui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"
	"github.com/charmbracelet/lipgloss/table"
)

var (
	clrCyan    = lipgloss.Color("6")
	clrBlue    = lipgloss.Color("12")
	clrGreen   = lipgloss.Color("2")
	clrHiGreen = lipgloss.Color("10")
	clrYellow  = lipgloss.Color("11")
	clrRed     = lipgloss.Color("9")
	clrGray    = lipgloss.Color("8")
)

var (
	TitleStyle   = lipgloss.NewStyle().Bold(true).Foreground(clrCyan)
	HeaderStyle  = lipgloss.NewStyle().Bold(true).Foreground(clrCyan)
	CommandStyle = lipgloss.NewStyle().Foreground(clrHiGreen)
	AliasStyle   = lipgloss.NewStyle().Foreground(clrYellow)
	HintStyle    = lipgloss.NewStyle().Faint(true)
	SuccessStyle = lipgloss.NewStyle().Foreground(clrGreen).Bold(true)
	WarningStyle = lipgloss.NewStyle().Foreground(clrYellow).Bold(true)
	ErrorStyle   = lipgloss.NewStyle().Foreground(clrRed).Bold(true)
	InfoStyle    = lipgloss.NewStyle().Foreground(clrBlue)
	MutedStyle   = lipgloss.NewStyle().Foreground(clrGray)
)

func Title(s string) string {
	return TitleStyle.Render(s)
}

func Header(s string) string {
	return HeaderStyle.Render(s)
}

func Command(s string) string {
	return CommandStyle.Render(s)
}

func Aliases(aliases []string) string {
	return AliasStyle.Render(fmt.Sprintf("(aliases: %s)", strings.Join(aliases, ", ")))
}

func Hint(s string) string {
	return HintStyle.Render(s)
}

func Success(s string) string {
	return SuccessStyle.Render(s)
}

func Warning(s string) string {
	return WarningStyle.Render(s)
}

func Error(s string) string {
	return ErrorStyle.Render(s)
}

func Info(s string) string {
	return InfoStyle.Render(s)
}

func Muted(s string) string {
	return MutedStyle.Render(s)
}

func RenderTable(headers []string, rows [][]string) string {
	t := table.New().Border(lipgloss.HiddenBorder()).Headers(headers...).Rows(rows...)
	t = t.StyleFunc(func(row, col int) lipgloss.Style {
		style := lipgloss.NewStyle().PaddingRight(2)
		if row == table.HeaderRow {
			return style.Bold(true).Faint(true)
		}
		return style
	})
	return t.String()
}
