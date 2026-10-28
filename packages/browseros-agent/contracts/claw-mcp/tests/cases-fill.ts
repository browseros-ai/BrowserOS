/** Fill is tested through every public MCP surface, with page-observed values
 * and events. These contracts intentionally avoid the implementation's CDP calls.
 *
 * Fill types a value and leaves the outcome to the page, which may keep, reformat or
 * reject it. act reads each filled field back as the snapshot shows it, so the caller
 * judges the result; only a fill that could not type (read-only, covered, focus
 * redirected) is an error.
 *
 * `run-legacy` drives the pre-handle call shape. Saved helpers on user machines
 * hold that shape as source and are hot-loaded verbatim, so it has to keep
 * behaving identically; these cases are what says so.
 */
import type { CaseContext, ContractCase } from './cases'
import { expectError, expectOk } from './helpers'

type Surface = 'act' | 'run' | 'run-legacy'

async function fieldRef(
  ctx: CaseContext,
  page: number,
  label: string,
): Promise<string> {
  const snapshot = expectOk(await ctx.mcp.callTool('snapshot', { page }))
  const line = snapshot.split('\n').find((line) => line.includes(`"${label}`))
  const ref = line?.match(/\[ref=(e\d+)\]/)?.[1]
  if (!ref) throw new Error(`No ref for ${label}`)
  return ref
}

async function fill(
  ctx: CaseContext,
  page: number,
  surface: Surface,
  ref: string,
  value: string,
) {
  if (surface === 'act') {
    return ctx.mcp.callTool('act', {
      page,
      kind: 'fill',
      fields: [{ ref, value }],
    })
  }
  // 'run-legacy' is the pre-handle call shape. Saved helpers on user machines
  // hold it as source, so it is a compatibility contract now, not a duplicate.
  const code =
    surface === 'run-legacy'
      ? `await browser.input(${page}).fill(${JSON.stringify(ref)}, ${JSON.stringify(value)})`
      : `await browser.page(${page}).fill(${JSON.stringify(ref)}, ${JSON.stringify(value)})`
  return ctx.mcp.callTool('run', { code })
}

/** The line act's fields readback shows for `ref`. */
function shownLine(text: string, ref: string): string | undefined {
  const start = text.indexOf(' fields]')
  if (start < 0) throw new Error(`No fields readback in: ${text}`)
  return text
    .slice(start)
    .split('\n')
    .find((line) => line.includes(`[ref=${ref}]`) || line.startsWith(`${ref}:`))
}

async function assertPage(
  ctx: CaseContext,
  page: number,
  expression: string,
): Promise<void> {
  // A failed assertion throws inside the page, which evaluate_in_page exposes as an MCP
  // error. This avoids substring checks that accidentally accept appended text.
  // The page's own world, because postconditions read the fixture's `fillEvents`.
  expectOk(
    await ctx.mcp.callTool('evaluate_in_page', {
      page,
      code: `if (!(${expression})) throw new Error("Fill postcondition failed"); return true`,
    }),
    expression,
  )
}

export const fillCases: ContractCase[] = [
  ...(['act', 'run', 'run-legacy'] as const).flatMap((surface) => [
    {
      name: `${surface}: fill replaces text, textarea, editable and native input types`,
      async run(ctx: CaseContext) {
        const page = await ctx.openPage(ctx.fixture('/fill.html'))
        for (const [label, id, value] of [
          ['Text field', 'text', 'new text 😀'],
          ['Multiline', 'multiline', 'line one\nline two'],
          ['Rich editor', 'rich', 'new rich\ntext'],
          ['Email field', 'email', 'new@example.com'],
          ['Number field', 'number', '123'],
          ['Password field', 'password', 'new password'],
        ]) {
          expectOk(
            await fill(
              ctx,
              page,
              surface,
              await fieldRef(ctx, page, label),
              value,
            ),
          )
          await assertPage(
            ctx,
            page,
            `(document.getElementById("${id}").value ?? document.getElementById("${id}").innerText) === ${JSON.stringify(value)}`,
          )
          await assertPage(
            ctx,
            page,
            `window.fillEvents.some(e => e.id === "${id}" && e.trusted)`,
          )
        }
      },
    },
    {
      name: `${surface}: fill clears prefilled text and contenteditable to empty`,
      async run(ctx: CaseContext) {
        const page = await ctx.openPage(ctx.fixture('/fill.html'))
        for (const [label, id] of [
          ['Text field', 'text'],
          ['Multiline', 'multiline'],
          ['Rich editor', 'rich'],
        ]) {
          expectOk(
            await fill(
              ctx,
              page,
              surface,
              await fieldRef(ctx, page, label),
              '',
            ),
          )
          await assertPage(
            ctx,
            page,
            `(document.getElementById("${id}").value ?? document.getElementById("${id}").textContent) === ""`,
          )
        }
      },
    },
    ...[
      ['Rejecting field', 'reject'],
      ['Reverting field', 'revert'],
    ].map(([label, id]) => ({
      name: `${surface}: fill leaves what a ${label.toLowerCase()} keeps to the page`,
      async run(ctx: CaseContext) {
        const page = await ctx.openPage(ctx.fixture('/fill.html'))
        const ref = await fieldRef(ctx, page, label)
        const text = expectOk(
          await fill(ctx, page, surface, ref, 'replacement'),
        )
        if (
          surface === 'act' &&
          !shownLine(text, ref)?.endsWith(': "original"')
        ) {
          throw new Error(`The readback hid what the field kept: ${text}`)
        }
        await assertPage(
          ctx,
          page,
          `document.getElementById("${id}").value === "original" && document.getElementById("other").value === "untouched"`,
        )
      },
    })),
    {
      name: `${surface}: fill leaves a value the page reformats to the page`,
      async run(ctx: CaseContext) {
        const page = await ctx.openPage(ctx.fixture('/fill.html'))
        const ref = await fieldRef(ctx, page, 'Phone field')
        const text = expectOk(await fill(ctx, page, surface, ref, '5551234567'))
        if (
          surface === 'act' &&
          !shownLine(text, ref)?.endsWith(': "(555) 123-4567"')
        ) {
          throw new Error(
            `The readback did not show the formatted value: ${text}`,
          )
        }
        await assertPage(
          ctx,
          page,
          'document.getElementById("phone").value === "(555) 123-4567"',
        )
      },
    },
    ...[
      ['Read only', 'readonly'],
      ['Disabled field', 'disabled'],
      ['Covered field', 'covered'],
      ['Focus redirect', 'redirect'],
    ].map(([label, id]) => ({
      name: `${surface}: fill rejects ${label.toLowerCase()} without false success`,
      async run(ctx: CaseContext) {
        const page = await ctx.openPage(ctx.fixture('/fill.html'))
        const result = await fill(
          ctx,
          page,
          surface,
          await fieldRef(ctx, page, label),
          'replacement',
        )
        expectError(result)
        await assertPage(
          ctx,
          page,
          `document.getElementById("${id}").value === "original" && document.getElementById("other").value === "untouched"`,
        )
      },
    })),
    {
      name: `${surface}: fill succeeds idempotently when the requested value already holds`,
      async run(ctx: CaseContext) {
        const page = await ctx.openPage(ctx.fixture('/fill.html'))
        expectOk(
          await fill(
            ctx,
            page,
            surface,
            await fieldRef(ctx, page, 'Rejecting field'),
            'original',
          ),
        )
        await assertPage(
          ctx,
          page,
          'document.getElementById("reject").value === "original"',
        )
      },
    },
    {
      name: `${surface}: fill reports a field the page removed as gone, not empty`,
      async run(ctx: CaseContext) {
        const page = await ctx.openPage(ctx.fixture('/fill.html'))
        const ref = await fieldRef(ctx, page, 'Detached field')
        const text = expectOk(await fill(ctx, page, surface, ref, ''))
        if (
          surface === 'act' &&
          shownLine(text, ref) !== `${ref}: not on the page now`
        ) {
          throw new Error(`The readback did not say the field is gone: ${text}`)
        }
        await assertPage(ctx, page, '!document.getElementById("detach")')
      },
    },
    {
      name: `${surface}: fill focuses a field inside a shadow root`,
      async run(ctx: CaseContext) {
        const page = await ctx.openPage(ctx.fixture('/fill.html'))
        expectOk(
          await fill(
            ctx,
            page,
            surface,
            await fieldRef(ctx, page, 'Shadow field'),
            'shadow text',
          ),
        )
        await assertPage(
          ctx,
          page,
          'document.getElementById("shadow-host").shadowRoot.querySelector("input").value === "shadow text"',
        )
      },
    },
    {
      name: `${surface}: fill focuses the referenced iframe instead of the parent field`,
      async run(ctx: CaseContext) {
        const page = await ctx.openPage(ctx.fixture('/form.html'))
        expectOk(
          await ctx.mcp.callTool('evaluate_in_page', {
            page,
            code: `const frame=document.createElement("iframe"); frame.src=${JSON.stringify(ctx.fixture2('/fill.html').replace('127.0.0.1', 'localhost'))}; document.body.prepend(frame); return new Promise(resolve => frame.onload=()=>resolve(true))`,
          }),
        )
        expectOk(
          await fill(
            ctx,
            page,
            surface,
            await fieldRef(ctx, page, 'Text field'),
            'frame text',
          ),
        )
        // Cross-site DOM access is blocked in the parent. The accessibility
        // snapshot independently exposes the child field's value.
        const snapshot = expectOk(await ctx.mcp.callTool('snapshot', { page }))
        const filledLine = snapshot
          .split('\n')
          .find((line) => /textbox "Text field\s*"/.test(line))
        if (!filledLine?.endsWith(': "frame text"')) {
          throw new Error(
            `The iframe did not receive the requested text: ${snapshot}`,
          )
        }
        expectOk(
          await fill(
            ctx,
            page,
            surface,
            await fieldRef(ctx, page, 'Text field'),
            '',
          ),
        )
        const cleared = expectOk(await ctx.mcp.callTool('snapshot', { page }))
        const emptyLine = cleared
          .split('\n')
          .find((line) => /textbox "Text field\s*"/.test(line))
        if (!emptyLine?.endsWith(']')) {
          throw new Error('The iframe field was not cleared')
        }
        await assertPage(
          ctx,
          page,
          'document.getElementById("name").value === ""',
        )
      },
    },
  ]),
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
    name: 'act: fill clear=false appends',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/fill.html'))
      for (const [label, id, value, expected] of [
        ['Text field', 'text', '-suffix', 'original-suffix'],
        ['Email field', 'email', '.test', 'old@example.com.test'],
        ['Number field', 'number', '3', '423'],
        ['Rich editor', 'rich', '-suffix', 'old content-suffix'],
      ]) {
        const ref = await fieldRef(ctx, page, label)
        expectOk(
          await ctx.mcp.callTool('act', {
            page,
            kind: 'fill',
            ref,
            value,
            clear: false,
          }),
        )
        await assertPage(
          ctx,
          page,
          `(document.getElementById("${id}").value ?? document.getElementById("${id}").innerText) === ${JSON.stringify(expected)}`,
        )
      }
    },
  },
]
