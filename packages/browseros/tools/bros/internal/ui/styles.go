package ui

import "github.com/charmbracelet/lipgloss"

var (
	TitleStyle    = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("12"))
	SubtitleStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("8"))

	SuccessStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("10"))
	WarningStyle = lipgloss.NewStyle().Foreground(lipgloss.Color("11"))
	ErrorStyle   = lipgloss.NewStyle().Foreground(lipgloss.Color("9"))
	MutedStyle   = lipgloss.NewStyle().Foreground(lipgloss.Color("8"))

	ModifiedPrefix = lipgloss.NewStyle().Foreground(lipgloss.Color("11")).Render("M")
	AddedPrefix    = lipgloss.NewStyle().Foreground(lipgloss.Color("10")).Render("A")
	DeletedPrefix  = lipgloss.NewStyle().Foreground(lipgloss.Color("9")).Render("D")
	SkippedPrefix  = lipgloss.NewStyle().Foreground(lipgloss.Color("8")).Render("~")

	ConflictBox = lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(lipgloss.Color("9")).
			Padding(0, 1)

	StatusBox = lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(lipgloss.Color("12")).
			Padding(0, 1)

	LabelStyle = lipgloss.NewStyle().Width(16).Foreground(lipgloss.Color("8"))
	ValueStyle = lipgloss.NewStyle().Bold(true)
)
