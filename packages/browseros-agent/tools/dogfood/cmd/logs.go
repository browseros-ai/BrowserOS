package cmd

import (
	"fmt"
	"io"

	"browseros-dogfood/config"
	"browseros-dogfood/proc"

	"github.com/spf13/cobra"
)

func init() {
	rootCmd.AddCommand(logsCmd)
}

var logsCmd = &cobra.Command{
	Use:     "logs",
	Short:   "Print dogfood log files",
	GroupID: groupInspect,
	RunE: func(cmd *cobra.Command, args []string) error {
		_, cfg, err := loadSelectedTargetConfigWithoutValidation()
		if err != nil {
			return err
		}
		return printLogs(cmd.OutOrStdout(), cfg)
	},
}

func printLogs(out io.Writer, cfg config.Config) error {
	logDir := cfg.LogDir()
	fmt.Fprintf(out, "%s %s\n", labelStyle.Sprint("Log directory:"), pathStyle.Sprint(logDir))
	files, err := proc.ListLogFiles(logDir)
	if err != nil {
		return err
	}
	if len(files) == 0 {
		fmt.Fprintln(out, dimStyle.Sprint("No log files found."))
		return nil
	}
	for _, file := range files {
		fmt.Fprintf(out, "%s %s\n", pathStyle.Sprint(file.Path), dimStyle.Sprintf("(%d bytes, modified %s)", file.Size, file.ModTime.Format("2006-01-02 15:04:05")))
	}
	return nil
}
