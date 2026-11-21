/**
 * act's fields readback: after a fill, every filled field as the snapshot shows it, so the caller
 * judges what the page made of each value. It survives a diff of "none" and a fill that stops
 * partway, and a ref only stands for the node it named when it was filled.
 */
import type { CaseContext, ContractCase } from './cases'
import { fieldRef, shownLine } from './cases-fill'
import { expectError, expectOk } from './helpers'

async function assertPage(
  ctx: CaseContext,
  page: number,
  expression: string,
): Promise<void> {
  expectOk(
    await ctx.mcp.callTool('evaluate_in_page', {
      page,
      code: `if (!(${expression})) throw new Error("Fill postcondition failed"); return true`,
    }),
    expression,
  )
}

export const fillReadbackCases: ContractCase[] = [
  {
    name: 'act: a multi-field fill carries on past a field the page reformats',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/fill.html'))
      const phone = await fieldRef(ctx, page, 'Phone field')
      const text = await fieldRef(ctx, page, 'Text field')
      const result = expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'fill',
          fields: [
            { ref: phone, value: '5551234567' },
            { ref: text, value: 'Ada' },
          ],
        }),
      )
      if (
        !shownLine(result, phone)?.endsWith(': "(555) 123-4567"') ||
        !shownLine(result, text)?.endsWith(': "Ada"')
      ) {
        throw new Error(`The readback missed a field: ${result}`)
      }
      await assertPage(
        ctx,
        page,
        'document.getElementById("text").value === "Ada"',
      )
    },
  },
  {
    name: 'act: the fields readback stays when the diff is off',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/fill.html'))
      const ref = await fieldRef(ctx, page, 'Verification code')
      const result = expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'fill',
          ref,
          value: '440212',
          diff: 'none',
        }),
      )
      if (
        result.includes(' diff]') ||
        !shownLine(result, ref)?.endsWith(': "440 212"')
      ) {
        throw new Error(`Expected only the fields readback: ${result}`)
      }
    },
  },
  {
    name: 'act: a fill that stops partway still shows every field',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/fill.html'))
      const text = await fieldRef(ctx, page, 'Text field')
      const readonly = await fieldRef(ctx, page, 'Read only')
      const other = await fieldRef(ctx, page, 'Other field')
      const result = expectError(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'fill',
          fields: [
            { ref: text, value: 'typed first' },
            { ref: readonly, value: 'refused' },
            { ref: other, value: 'never reached' },
          ],
        }),
      )
      if (
        !shownLine(result, text)?.endsWith(': "typed first"') ||
        !shownLine(result, readonly)?.endsWith(': "original"') ||
        !shownLine(result, other)?.endsWith(': "untouched"')
      ) {
        throw new Error(`The error hid what the fields hold: ${result}`)
      }
    },
  },
  {
    name: 'act: after a fill navigates, its reused ref reads as gone, not as the new page',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/fill.html'))
      const ref = await fieldRef(ctx, page, 'Navigating field')
      const result = expectOk(
        await ctx.mcp.callTool('act', { page, kind: 'fill', ref, value: 'go' }),
      )
      // The reloaded page numbers its refs the same way, so the same ref names the new
      // document's empty copy of this field. It is not the field that was filled.
      if (shownLine(result, ref) !== `${ref}: not on the page now`) {
        throw new Error(
          `The readback showed another node under ${ref}: ${result}`,
        )
      }
      if ((await fieldRef(ctx, page, 'Navigating field')) !== ref) {
        throw new Error(
          `The reloaded page did not reuse ${ref}; nothing was proved`,
        )
      }
    },
  },
]
