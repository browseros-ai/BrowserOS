# Project Instructions

## Docs Image Workflow

When updating documentation that involves new screenshots or images:

1. Prompt the user to copy the image to their clipboard (Cmd+C)
2. Run: `python scripts/save_clipboard.py <target_path>`
3. Example: `python scripts/save_clipboard.py docs/images/agent-step.png`

This saves the clipboard image directly to the docs folder without manual file management.
