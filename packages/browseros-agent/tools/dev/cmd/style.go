package cmd

import "github.com/fatih/color"

var (
	headerStyle  = color.New(color.Bold, color.FgCyan)
	commandStyle = color.New(color.FgHiGreen)
	successStyle = color.New(color.FgGreen, color.Bold)
	warnStyle    = color.New(color.FgYellow, color.Bold)
	labelStyle   = color.New(color.Bold)
	pathStyle    = color.New(color.FgCyan)
	dimStyle     = color.New(color.Faint)
)
