---
name: write-docs
description: Write BrowserOS feature documentation. Use when the user wants to create or update documentation for a BrowserOS feature. This skill explores the codebase to understand features and writes concise Mintlify MDX docs.
allowed-tools: Read, Grep, Glob, Bash, Task, Write, Edit
---

# Write BrowserOS Documentation

This skill helps write documentation for BrowserOS features. It follows a structured workflow to create high-quality, concise documentation pages.

## Workflow

### Step 1: Understand the Feature

Before writing documentation, explore the codebase to understand the feature:

1. **Ask the user** which feature they want to document
2. **Search the codebase** at `../browseros-server` (sibling directory) to find relevant code:
   - Use `Grep` to search for feature-related keywords
   - Use `Glob` to find relevant files
   - Read key files to understand how the feature works
3. **Identify key aspects**:
   - What problem does this feature solve?
   - How does the user enable/configure it?
   - What are the main use cases?

### Step 2: Gather Screenshots

Ask the user to provide screenshots for the documentation:

1. Tell the user: "Please copy a screenshot to your clipboard (Cmd+C) that shows [specific UI element]"
2. Run: `python scripts/save_clipboard.py docs/images/<feature-name>.png`
3. Repeat for any additional screenshots needed

### Step 3: Write the Documentation

Create the MDX file at `docs/features/<feature-name>.mdx` (or appropriate location) following this structure:

```mdx
---
title: "Feature Name"
description: "One sentence describing what this feature does"
---

[Opening paragraph: 1-2 sentences explaining what the feature does and why it matters]

## How It Works

[Explain the core mechanics in 2-3 paragraphs max]

## Getting Started

[Step-by-step instructions to use the feature]

1. Step one
2. Step two
3. Step three

## [Optional: Additional Sections]

[Only if necessary - keep the doc to ONE PAGE maximum]
```

### Step 4: Update Navigation

Add the new page to `docs/docs.json` under the appropriate group in the `navigation.groups` array.

### Step 5: Preview

Tell the user to run `mint dev` in the `docs/` directory to preview the documentation.

## Documentation Style Guide

- **Concise**: Maximum one page length
- **Clear**: Write for first-time BrowserOS users
- **Practical**: Focus on how to use the feature, not internal implementation details
- **Visual**: Use screenshots to show, not just tell
- **No fluff**: Skip unnecessary introductions or conclusions

## Example: Ad Blocking Doc Structure

```mdx
---
title: "Ad Blocking"
description: "BrowserOS blocks 10x more ads than Chrome out of the box"
---

BrowserOS ships with built-in ad blocking that works immediately—no extensions required.

## How It Works

[2-3 paragraphs explaining the mechanics]

## BrowserOS vs Chrome

[Comparison with data/screenshots]

## What This Means

[1 paragraph on the practical benefits]
```

## Key Directories

- **Docs location**: `docs/`
- **Images**: `docs/images/`
- **Feature code**: `../browseros-server/` (sibling directory)
- **Config**: `docs/docs.json`

## Core Features to Document

The user mentioned these features need documentation:
1. BrowserOS MCP Server
2. Connecting other MCPs to BrowserOS assistant
3. Scheduled tasks
4. [Additional features discovered in codebase]
