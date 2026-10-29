/** Real input contracts for the small SDK fixes. Native state and application
 * event records distinguish delivered input from a tool merely returning OK. */
import { expect } from 'bun:test'
import type { CaseContext, ContractCase } from './cases'
import { fieldRef } from './cases-fill'
import { expectOk } from './helpers'
import {
  errorIncludes,
  evalValue,
  inputCall,
  runValue,
  type Surface,
} from './sdk-helpers'

const nativeInputs = [
  ['Date field', 'date', '2026-10-29'],
  ['Time field', 'time', '14:30'],
  ['Datetime field', 'datetime-local', '2026-10-29T14:30'],
  ['Month field', 'month', '2026-10'],
  ['Week field', 'week', '2026-W44'],
  ['Color field', 'color', '#ff8800'],
  ['Range field', 'range', '75'],
] as const

function checkboxCases(surface: Surface): ContractCase[] {
  return [
    {
      name: `${surface}: check and uncheck verify a native checkbox`,
      async run(ctx) {
        const page = await ctx.openPage(ctx.fixture('/sdk-input.html'))
        for (const [method, checked] of [
          ['check', true],
          ['uncheck', false],
        ] as const) {
          expectOk(
            await inputCall(
              ctx,
              page,
              surface,
              method,
              await fieldRef(ctx, page, 'Native checkbox'),
            ),
          )
          expect(
            await evalValue(
              ctx,
              page,
              'return document.getElementById("native").checked',
            ),
          ).toBe(checked)
        }
      },
    },
    {
      name: `${surface}: check leaves an enabled ARIA switch on without clicking`,
      async run(ctx) {
        const page = await ctx.openPage(ctx.fixture('/sdk-input.html'))
        expectOk(
          await inputCall(
            ctx,
            page,
            surface,
            'check',
            await fieldRef(ctx, page, 'Enabled switch'),
          ),
        )
        expect(
          await evalValue(
            ctx,
            page,
            'return { checked: document.getElementById("switch").getAttribute("aria-checked"), clicks: window.switchClicks }',
          ),
        ).toEqual({ checked: 'true', clicks: 0 })
      },
    },
    {
      name: `${surface}: uncheck refuses a checked radio and leaves it checked`,
      async run(ctx) {
        const page = await ctx.openPage(ctx.fixture('/sdk-input.html'))
        errorIncludes(
          await inputCall(
            ctx,
            page,
            surface,
            'uncheck',
            await fieldRef(ctx, page, 'Checked radio'),
          ),
          'cannot be unchecked',
        )
        expect(
          await evalValue(
            ctx,
            page,
            'return document.getElementById("radio").checked',
          ),
        ).toBe(true)
      },
    },
    {
      name: `${surface}: check and uncheck follow a label ref to its hidden checkbox`,
      async run(ctx) {
        const page = await ctx.openPage(ctx.fixture('/sdk-input.html'))
        const snap = expectOk(await ctx.mcp.callTool('snapshot', { page }))
        // Visible labels normally share the input ref; this one must target the label itself.
        const line = snap
          .split('\n')
          .find((line) => line.includes('LabelText "Styled checkbox label"'))
        const ref = line?.match(/\[ref=(e\d+)\]/)?.[1]
        if (!ref) throw new Error(`No label ref in: ${snap}`)
        for (const [method, checked] of [
          ['check', true],
          ['uncheck', false],
        ] as const) {
          expectOk(await inputCall(ctx, page, surface, method, ref))
          expect(
            await evalValue(
              ctx,
              page,
              'return document.getElementById("label-control").checked',
            ),
          ).toBe(checked)
        }
      },
    },
    {
      name: `${surface}: check errors when a click does not change checked state`,
      async run(ctx) {
        const page = await ctx.openPage(ctx.fixture('/sdk-input.html'))
        errorIncludes(
          await inputCall(
            ctx,
            page,
            surface,
            'check',
            await fieldRef(ctx, page, 'Inert checkbox'),
          ),
          'did not make it checked',
        )
        expect(
          await evalValue(
            ctx,
            page,
            'return document.getElementById("inert").getAttribute("aria-checked")',
          ),
        ).toBe('false')
      },
    },
    {
      name: `${surface}: uncheck leaves an unchecked indeterminate checkbox off, then check turns it on`,
      async run(ctx) {
        const page = await ctx.openPage(ctx.fixture('/sdk-input.html'))
        const state =
          'const box = document.getElementById("mixed"); return { checked: box.checked, indeterminate: box.indeterminate }'
        expectOk(
          await inputCall(
            ctx,
            page,
            surface,
            'uncheck',
            await fieldRef(ctx, page, 'Mixed checkbox'),
          ),
        )
        // Not clicked: the box keeps its mixed look and stays off.
        expect(await evalValue(ctx, page, state)).toEqual({
          checked: false,
          indeterminate: true,
        })
        expectOk(
          await inputCall(
            ctx,
            page,
            surface,
            'check',
            await fieldRef(ctx, page, 'Mixed checkbox'),
          ),
        )
        expect(await evalValue(ctx, page, state)).toEqual({
          checked: true,
          indeterminate: false,
        })
      },
    },
  ]
}

function fillCases(surface: Surface): ContractCase[] {
  // Keep ordinary native controls here: if snapshot omits the input's ref, that
  // is a real end-to-end failure, not a reason to relabel it as an ARIA textbox.
  return [
    ...nativeInputs.map(
      ([label, id, value]): ContractCase => ({
        name: `${surface}: fill sets ${id} and fires input and change with the value`,
        async run(ctx) {
          const page = await ctx.openPage(ctx.fixture('/sdk-fill.html'))
          expectOk(
            await inputCall(
              ctx,
              page,
              surface,
              'fill',
              await fieldRef(ctx, page, label),
              value,
            ),
          )
          expect(
            await evalValue(
              ctx,
              page,
              `return document.getElementById(${JSON.stringify(id)}).value`,
            ),
          ).toBe(value)
          expect(
            await evalValue(
              ctx,
              page,
              `return window.nativeEvents.filter(e => e.id === ${JSON.stringify(id)})`,
            ),
          ).toEqual([
            { id, kind: 'input', value },
            { id, kind: 'change', value },
          ])
        },
      }),
    ),
    // The browser rewrites some values it accepts; fill must not mistake that for a refusal.
    ...(
      [
        [
          'Datetime field',
          'datetime-local',
          '2026-10-29T14:30:00',
          '2026-10-29T14:30',
        ],
        ['Range field', 'range', '75.0', '75'],
      ] as const
    ).map(
      ([label, id, value, stored]): ContractCase => ({
        name: `${surface}: fill accepts ${id} ${value}, which the browser stores as ${stored}`,
        async run(ctx) {
          const page = await ctx.openPage(ctx.fixture('/sdk-fill.html'))
          expectOk(
            await inputCall(
              ctx,
              page,
              surface,
              'fill',
              await fieldRef(ctx, page, label),
              value,
            ),
          )
          expect(
            await evalValue(
              ctx,
              page,
              `return document.getElementById(${JSON.stringify(id)}).value`,
            ),
          ).toBe(stored)
        },
      }),
    ),
    {
      name: `${surface}: fill rejects a range value outside its bounds without changing it`,
      async run(ctx) {
        const page = await ctx.openPage(ctx.fixture('/sdk-fill.html'))
        const before = await evalValue(
          ctx,
          page,
          'return document.getElementById("range").value',
        )
        errorIncludes(
          await inputCall(
            ctx,
            page,
            surface,
            'fill',
            await fieldRef(ctx, page, 'Range field'),
            '150',
          ),
          'a number from',
        )
        expect(
          await evalValue(
            ctx,
            page,
            'return document.getElementById("range").value',
          ),
        ).toBe(before)
      },
    },
    {
      name: `${surface}: fill rejects an invalid date format without changing the field`,
      async run(ctx) {
        const page = await ctx.openPage(ctx.fixture('/sdk-fill.html'))
        errorIncludes(
          await inputCall(
            ctx,
            page,
            surface,
            'fill',
            await fieldRef(ctx, page, 'Date field'),
            '29/10/2026',
          ),
          'YYYY-MM-DD',
        )
        expect(
          await evalValue(
            ctx,
            page,
            'return { value: document.getElementById("date").value, events: window.nativeEvents }',
          ),
        ).toEqual({ value: '2020-01-02', events: [] })
      },
    },
  ]
}

function dragCases(surface: Surface): ContractCase[] {
  return ['script', 'html'].map(
    (kind): ContractCase => ({
      name: `${surface}: drag ${kind === 'script' ? 'holds the button through mousemove and drops at the target' : 'delivers HTML5 dragstart dragover and DataTransfer drop'}`,
      async run(ctx) {
        const page = await ctx.openPage(ctx.fixture('/sdk-drag.html'))
        const from = await fieldRef(
          ctx,
          page,
          kind === 'script' ? 'Script source' : 'HTML source',
        )
        const to = await fieldRef(
          ctx,
          page,
          kind === 'script' ? 'Script target' : 'HTML target',
        )
        if (surface === 'act') {
          expectOk(
            await ctx.mcp.callTool('act', {
              page,
              kind: 'drag',
              ref: from,
              targetRef: to,
            }),
          )
        } else {
          await runValue(
            ctx,
            page,
            `await p.drag(${JSON.stringify(from)}, ${JSON.stringify(to)})`,
          )
        }
        await assertDrag(ctx, page, kind)
      },
    }),
  )
}

async function assertDrag(
  ctx: CaseContext,
  page: number,
  kind: string,
): Promise<void> {
  if (kind === 'html') {
    const state = await evalValue<{
      starts: number
      overs: number
      dropped: string
    }>(ctx, page, 'return window.htmlDrag')
    expect(state.starts).toBeGreaterThan(0)
    expect(state.overs).toBeGreaterThan(0)
    expect(state.dropped).toBe('html-source')
  } else {
    const state = await evalValue<{
      down: boolean
      heldMoves: number
      up: boolean
      drop: { x: number; y: number } | null
    }>(ctx, page, 'return window.scriptDrag')
    expect(state.down).toBe(true)
    expect(state.heldMoves).toBeGreaterThan(0)
    expect(state.up).toBe(true)
    expect(state.drop).not.toBeNull()
    const target = await center(ctx, page, 'script-target')
    expect(state.drop?.x).toBeCloseTo(target.x, 0)
    expect(state.drop?.y).toBeCloseTo(target.y, 0)
  }
}

async function center(
  ctx: CaseContext,
  page: number,
  id: string,
): Promise<{ x: number; y: number }> {
  return evalValue<{ x: number; y: number }>(
    ctx,
    page,
    `const r = document.getElementById(${JSON.stringify(id)}).getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }`,
  )
}

export const sdkInputCases: ContractCase[] = [
  {
    name: 'run: insertText preserves textarea newlines without Enter or submit',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/sdk-input.html'))
      const ref = await fieldRef(ctx, page, 'Literal multiline')
      await runValue(
        ctx,
        page,
        `await p.click(${JSON.stringify(ref)}); await p.insertText('a\\nb')`,
      )
      expect(
        await evalValue(
          ctx,
          page,
          'return { value: document.getElementById("multiline").value, submits: window.submits, enters: window.enterKeys }',
        ),
      ).toEqual({ value: 'a\nb', submits: 0, enters: 0 })
    },
  },
  // insertText exists only on the SDK; granular type still exercises key semantics.
  ...(['act', 'run'] as const).map(
    (surface): ContractCase => ({
      name: `${surface}: type newline still presses Enter and submits the form`,
      async run(ctx) {
        const page = await ctx.openPage(ctx.fixture('/sdk-input.html'))
        const ref = await fieldRef(ctx, page, 'Submit field')
        expectOk(await ctx.mcp.callTool('act', { page, kind: 'click', ref }))
        if (surface === 'act') {
          expectOk(
            await ctx.mcp.callTool('act', { page, kind: 'type', text: 'a\n' }),
          )
        } else {
          await runValue(ctx, page, "await p.type('a\\n')")
        }
        expect(
          await evalValue(
            ctx,
            page,
            'return { value: document.getElementById("submit-field").value, submits: window.submits, enters: window.enterKeys }',
          ),
        ).toEqual({ value: 'a', submits: 1, enters: 1 })
      },
    }),
  ),
  ...(['act', 'run'] as const).flatMap((surface) => [
    ...checkboxCases(surface),
    ...fillCases(surface),
    ...dragCases(surface),
    {
      name: `${surface}: dragAt holds the button and records the drop coordinates`,
      async run(ctx: CaseContext) {
        const page = await ctx.openPage(ctx.fixture('/sdk-drag.html'))
        const from = await center(ctx, page, 'script-source')
        const to = await center(ctx, page, 'script-target')
        if (surface === 'act') {
          expectOk(
            await ctx.mcp.callTool('act', {
              page,
              kind: 'drag_at',
              startX: from.x,
              startY: from.y,
              endX: to.x,
              endY: to.y,
            }),
          )
        } else {
          await runValue(
            ctx,
            page,
            `await p.dragAt(${from.x}, ${from.y}, ${to.x}, ${to.y})`,
          )
        }
        await assertDrag(ctx, page, 'script')
      },
    },
    {
      name: `${surface}: dragAt moves a pointer-capture handle by the drag distance`,
      async run(ctx: CaseContext) {
        const page = await ctx.openPage(ctx.fixture('/sdk-drag.html'))
        const from = await center(ctx, page, 'capture-handle')
        const to = { x: from.x + 80, y: from.y + 60 }
        if (surface === 'act') {
          expectOk(
            await ctx.mcp.callTool('act', {
              page,
              kind: 'drag_at',
              startX: from.x,
              startY: from.y,
              endX: to.x,
              endY: to.y,
            }),
          )
        } else {
          await runValue(
            ctx,
            page,
            `await p.dragAt(${from.x}, ${from.y}, ${to.x}, ${to.y})`,
          )
        }
        const state = await evalValue<{
          capturedMoves: number
          released: boolean
          hadCapture: boolean
          moved: { left: number; top: number } | null
        }>(ctx, page, 'return window.pointerDrag')
        expect(state.capturedMoves).toBeGreaterThan(0)
        expect(state).toMatchObject({ released: true, hadCapture: true })
        expect(state.moved?.left).toBeCloseTo(20 + 80, 0)
        expect(state.moved?.top).toBeCloseTo(20 + 60, 0)
      },
    },
  ]),
]
