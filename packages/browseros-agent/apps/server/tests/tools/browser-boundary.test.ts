import { describe, it } from 'bun:test'
import assert from 'node:assert'
import { existsSync } from 'node:fs'
import { join } from 'node:path'
import { BROWSER_TOOLS } from '../../src/tools/browser/registry'

const compactBrowserToolFiles = [
  'act.ts',
  'diff.ts',
  'download.ts',
  'evaluate.ts',
  'framework.ts',
  'grep.ts',
  'navigate.ts',
  'output-file.ts',
  'pdf.ts',
  'read.ts',
  'register.ts',
  'registry.ts',
  'run.ts',
  'screenshot.ts',
  'snapshot.ts',
  'tab-groups.ts',
  'tabs.ts',
  'trust-boundary.ts',
  'upload.ts',
  'wait.ts',
  'windows.ts',
]

const legacyOnlyToolNames = [
  'get_bookmarks',
  'get_dom',
  'search_history',
  'click',
  'list_pages',
  'save_pdf',
  'take_snapshot',
  'group_tabs',
  'list_windows',
  'create_window',
  'create_hidden_window',
  'close_window',
  'activate_window',
  'set_window_visibility',
]

describe('browser tool boundary', () => {
  it('keeps the compact browser tools under src/tools/browser', () => {
    const toolsDir = join(import.meta.dir, '../../src/tools')

    for (const file of compactBrowserToolFiles) {
      assert.ok(
        existsSync(join(toolsDir, 'browser', file)),
        `Expected browser/${file}`,
      )
    }
  })

  it('does not keep legacy browser tool modules', () => {
    const toolsDir = join(import.meta.dir, '../../src/tools')

    assert.ok(!existsSync(join(toolsDir, 'legacy')))
  })

  it('does not register the legacy-only browser tool names', () => {
    const activeNames = new Set(BROWSER_TOOLS.map((tool) => tool.name))

    for (const name of legacyOnlyToolNames) {
      assert.ok(!activeNames.has(name), `Unexpected active tool ${name}`)
    }
  })
})
