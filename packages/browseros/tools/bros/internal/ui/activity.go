package ui

import (
	"fmt"
	"strings"
)

var (
	StepPrefix  = TitleStyle.Render(">")
	InfoPrefix  = SubtitleStyle.Render("i")
	OkPrefix    = SuccessStyle.Render("✓")
	WarnPrefix  = WarningStyle.Render("!")
	ErrorPrefix = ErrorStyle.Render("x")
)

type Activity struct {
	Verbose bool
}

func NewActivity(verbose bool) *Activity {
	return &Activity{Verbose: verbose}
}

func (a *Activity) Header(title string) {
	fmt.Println(TitleStyle.Render(title))
	fmt.Println()
}

func (a *Activity) Step(format string, args ...any) {
	fmt.Printf("  %s %s\n", StepPrefix, fmt.Sprintf(format, args...))
}

func (a *Activity) Info(format string, args ...any) {
	fmt.Printf("  %s %s\n", InfoPrefix, fmt.Sprintf(format, args...))
}

func (a *Activity) Detail(format string, args ...any) {
	if !a.Verbose {
		return
	}
	fmt.Printf("    %s %s\n", InfoPrefix, fmt.Sprintf(format, args...))
}

func (a *Activity) Success(format string, args ...any) {
	fmt.Printf("  %s %s\n", OkPrefix, fmt.Sprintf(format, args...))
}

func (a *Activity) Warn(format string, args ...any) {
	fmt.Printf("  %s %s\n", WarnPrefix, fmt.Sprintf(format, args...))
}

func (a *Activity) Error(format string, args ...any) {
	fmt.Printf("  %s %s\n", ErrorPrefix, fmt.Sprintf(format, args...))
}

func (a *Activity) Divider() {
	fmt.Println(MutedStyle.Render(strings.Repeat("-", 58)))
}
