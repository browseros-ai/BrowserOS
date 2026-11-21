/**
 * A `<label>` is clickable because clicking it clicks its control, and Chromium says so. A label
 * whose control is in the snapshot is that control's target, not a second one; a label whose
 * control is hidden (a styled checkbox) is the only target, so it keeps a ref named by its text.
 */

import type { CaseContext, ContractCase } from './cases'
import { expectOk } from './helpers'

function lineWith(snapshot: string, pattern: RegExp): string {
  const line = snapshot.split('\n').find((line) => pattern.test(line))
  if (!line) throw new Error(`No line matches ${pattern}: ${snapshot}`)
  return line
}

export const labelCases: ContractCase[] = [
  {
    name: 'snapshot: labels share their control ref; a hidden control leaves its label one',
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(ctx.fixture('/labels.html'))
      const snapshot = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      for (const name of ['Email address', 'Wrapped name', 'Pointer label']) {
        lineWith(snapshot, new RegExp(`textbox "${name}".*\\[ref=e\\d+\\]`))
      }
      const labelRefs = snapshot
        .split('\n')
        .filter((line) => line.includes('LabelText') && line.includes('[ref='))
      if (
        labelRefs.length !== 1 ||
        !labelRefs[0]?.includes('LabelText "Remember me"')
      ) {
        throw new Error(`Expected one label ref, for Remember me: ${snapshot}`)
      }
      const ref = labelRefs[0].match(/\[ref=(e\d+)\]/)?.[1]
      expectOk(await ctx.mcp.callTool('act', { page, kind: 'click', ref }))
      expectOk(
        await ctx.mcp.callTool('evaluate_in_page', {
          page,
          code: 'if (!document.getElementById("remember").checked) throw new Error("not checked"); return true',
        }),
        'the label click checks the hidden box',
      )
    },
  },
]
