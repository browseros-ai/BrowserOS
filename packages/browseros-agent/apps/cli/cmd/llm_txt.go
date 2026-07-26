package cmd

import _ "embed"

// llmTxtGuide is the agent usage guide printed by `--llm-txt`, embedded so it ships in the binary.
//
//go:embed llm_txt.md
var llmTxtGuide string
