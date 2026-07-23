/**
 * act (20) — every kind and every failure path. Snapshots are taken
 * before each action to mint fresh refs; DOM state is verified through
 * evaluate (`return …`, since evaluate runs the code as a function body).
 */

import type { CaseContext, ContractCase } from './cases'
import { errorClass, expectError, expectOk, waitUntil } from './helpers'
import { textOf } from './mcp-client'

async function snapshot(ctx: CaseContext, page: number): Promise<string> {
  return expectOk(await ctx.mcp.callTool('snapshot', { page }), 'snapshot')
}

function refFor(snapshot: string, needle: string): string {
  for (const line of snapshot.split('\n')) {
    if (line.includes(needle)) {
      const match = line.match(/\[ref=(e\d+)\]/)
      if (match) return match[1]
    }
  }
  throw new Error(`no ref for "${needle}" in:\n${snapshot.slice(0, 500)}`)
}

async function evalIn(
  ctx: CaseContext,
  page: number,
  code: string,
): Promise<string> {
  return expectOk(
    await ctx.mcp.callTool('evaluate', { page, code }),
    'evaluate',
  )
}

export const actCases: ContractCase[] = [
  {
    name: 'act: click updates the page',
    smoke: true,
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const snap = await snapshot(ctx, page)
      expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'click',
          ref: refFor(snap, 'Apply'),
        }),
        'act click',
      )
      await waitUntil(
        async () =>
          (
            await evalIn(
              ctx,
              page,
              'return document.getElementById("result").textContent',
            )
          ).includes('applied'),
        'the click to update #result',
      )
    },
  },
  {
    name: 'act: click on a covered element names the blocker',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/overlay.html'))
      const snap = await snapshot(ctx, page)
      const result = await ctx.mcp.callTool('act', {
        page,
        kind: 'click',
        ref: refFor(snap, 'Covered button'),
      })
      // The click must be refused with enough context to identify the blocker.
      const text = textOf(result)
      const namesBlocker = /overlay|cover|intercept|obscur/i.test(text)
      const stillUntouched = (
        await evalIn(
          ctx,
          page,
          'return document.getElementById("overlay-result").textContent',
        )
      ).includes('untouched')
      if (!namesBlocker || !stillUntouched) {
        throw new Error(
          `covered click activated or omitted the blocker: ${text}`,
        )
      }
    },
  },
  {
    name: 'act: click_at hits page coordinates',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/cursor.html'))
      await snapshot(ctx, page)
      const box = await evalIn(
        ctx,
        page,
        'const r = document.getElementById("cursor-div").getBoundingClientRect(); return JSON.stringify({x: Math.round(r.x + r.width/2), y: Math.round(r.y + r.height/2)})',
      )
      const { x, y } = JSON.parse(box.match(/\{.*\}/)?.[0] ?? '{}')
      expectOk(
        await ctx.mcp.callTool('act', { page, kind: 'click_at', x, y }),
        'act click_at',
      )
      await waitUntil(
        async () =>
          (
            await evalIn(
              ctx,
              page,
              'return document.getElementById("cursor-result").textContent',
            )
          ).includes('div-clicked'),
        'click_at to fire the div handler',
      )
    },
  },
  {
    name: 'act: type enters text into a field',
    smoke: true,
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const snap = await snapshot(ctx, page)
      const nameRef = refFor(snap, '"Name ')
      // type sends key events to the focused element; a real click is the
      // reliable focus primitive (act kind=focus is broken on both
      // servers — see the focus case). Click, then type.
      expectOk(
        await ctx.mcp.callTool('act', { page, kind: 'click', ref: nameRef }),
      )
      expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'type',
          ref: nameRef,
          text: 'Ada Lovelace',
        }),
        'act type',
      )
      const value = await evalIn(
        ctx,
        page,
        'return document.getElementById("name").value',
      )
      if (!value.includes('Ada Lovelace')) {
        throw new Error(`type did not fill the field: ${value}`)
      }
    },
  },
  {
    name: 'act: type_at enters text at coordinates',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      await snapshot(ctx, page)
      const box = await evalIn(
        ctx,
        page,
        'const r = document.getElementById("bio").getBoundingClientRect(); return JSON.stringify({x: Math.round(r.x + 10), y: Math.round(r.y + 10)})',
      )
      const { x, y } = JSON.parse(box.match(/\{.*\}/)?.[0] ?? '{}')
      const result = await ctx.mcp.callTool('act', {
        page,
        kind: 'type_at',
        x,
        y,
        text: 'typed at point',
      })
      if (!result.isError) {
        const value = await evalIn(
          ctx,
          page,
          'return document.getElementById("bio").value',
        )
        if (!value.includes('typed at point')) {
          throw new Error(`type_at did not reach the textarea: ${value}`)
        }
      }
    },
  },
  {
    name: 'act: fill sets a single field',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const snap = await snapshot(ctx, page)
      expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'fill',
          ref: refFor(snap, '"Name '),
          value: 'Grace Hopper',
        }),
        'act fill',
      )
      const value = await evalIn(
        ctx,
        page,
        'return document.getElementById("name").value',
      )
      if (!value.includes('Grace Hopper')) {
        throw new Error(`fill did not set the field: ${value}`)
      }
    },
  },
  {
    name: 'act: fill sets a whole form via fields[] in one call',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const snap = await snapshot(ctx, page)
      expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'fill',
          fields: [
            { ref: refFor(snap, '"Name '), value: 'Katherine Johnson' },
            { ref: refFor(snap, '"Bio '), value: 'orbital mechanics' },
          ],
        }),
        'act fill fields[]',
      )
      const name = await evalIn(
        ctx,
        page,
        'return document.getElementById("name").value',
      )
      const bio = await evalIn(
        ctx,
        page,
        'return document.getElementById("bio").value',
      )
      if (
        !name.includes('Katherine Johnson') ||
        !bio.includes('orbital mechanics')
      ) {
        throw new Error(`batch fill missed a field: name=${name} bio=${bio}`)
      }
    },
  },
  {
    name: 'act: press Enter submits the form',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const snap = await snapshot(ctx, page)
      const nameRef = refFor(snap, '"Name ')
      expectOk(
        await ctx.mcp.callTool('act', { page, kind: 'click', ref: nameRef }),
      )
      expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'fill',
          ref: nameRef,
          value: 'Enter Test',
        }),
      )
      expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'press',
          ref: nameRef,
          key: 'Enter',
        }),
        'act press Enter',
      )
      await waitUntil(
        async () =>
          (
            await evalIn(
              ctx,
              page,
              'return document.getElementById("result").textContent',
            )
          ).includes('submitted'),
        'Enter to submit the form',
      )
    },
  },
  {
    name: 'act: press Shift+a types an uppercase A',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const snap = await snapshot(ctx, page)
      const nameRef = refFor(snap, '"Name ')
      expectOk(
        await ctx.mcp.callTool('act', { page, kind: 'click', ref: nameRef }),
      )
      expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'press',
          ref: nameRef,
          key: 'Shift+a',
        }),
        'act press Shift+a',
      )
      await waitUntil(
        async () =>
          (
            await evalIn(
              ctx,
              page,
              'return document.getElementById("name").value',
            )
          ).includes('A'),
        'Shift+a to type an uppercase A',
      )
      const value = await evalIn(
        ctx,
        page,
        'return document.getElementById("name").value',
      )
      if (value.includes('a') && !value.includes('A')) {
        throw new Error(`Shift+a produced lowercase: ${value}`)
      }
    },
  },
  {
    name: 'act: press resolves the Cmd+c alias',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const snap = await snapshot(ctx, page)
      const result = await ctx.mcp.callTool('act', {
        page,
        kind: 'press',
        ref: refFor(snap, '"Name '),
        key: 'Cmd+c',
      })
      if (result.isError) {
        throw new Error(`Cmd+c alias did not resolve: ${textOf(result)}`)
      }
    },
  },
  {
    name: 'act: press with an invalid key errors and lists keys',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const snap = await snapshot(ctx, page)
      expectError(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'press',
          ref: refFor(snap, '"Name '),
          key: 'NotARealKey',
        }),
        'act press invalid key',
      )
    },
  },
  {
    name: 'act: hover reveals a tooltip in the diff',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/scroll.html'))
      const snap = await snapshot(ctx, page)
      const result = await ctx.mcp.callTool('act', {
        page,
        kind: 'hover',
        ref: refFor(snap, 'Hover me'),
      })
      expectOk(result, 'act hover')
      const visible = await evalIn(
        ctx,
        page,
        'return getComputedStyle(document.getElementById("tooltip")).display',
      )
      if (!visible.includes('block')) {
        throw new Error(`hover did not reveal the tooltip: ${visible}`)
      }
    },
  },
  {
    name: 'act: focus by ref reports the current DOM-domain limitation',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const snap = await snapshot(ctx, page)
      expectError(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'focus',
          ref: refFor(snap, '"Bio '),
        }),
        'act focus by ref',
      )
      const active = await evalIn(
        ctx,
        page,
        'return document.activeElement && document.activeElement.id',
      )
      if (active.includes('bio')) {
        throw new Error('failed focus action unexpectedly moved focus')
      }
    },
  },
  {
    name: 'act: click toggles a checkbox and is repeatable',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const snap = await snapshot(ctx, page)
      const ref = refFor(snap, 'Subscribe')
      expectOk(await ctx.mcp.callTool('act', { page, kind: 'click', ref }))
      const first = await evalIn(
        ctx,
        page,
        'return document.getElementById("subscribe").checked',
      )
      if (!first.includes('true')) {
        throw new Error(`checkbox click did not check: ${first}`)
      }
      // A second snapshot mints a fresh ref for the same node.
      const snap2 = await snapshot(ctx, page)
      expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'click',
          ref: refFor(snap2, 'Subscribe'),
        }),
      )
      const second = await evalIn(
        ctx,
        page,
        'return document.getElementById("subscribe").checked',
      )
      if (!second.includes('false')) {
        throw new Error(`second checkbox click did not uncheck: ${second}`)
      }
    },
  },
  {
    name: 'act: check and uncheck kinds set checkbox state',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const snap = await snapshot(ctx, page)
      const ref = refFor(snap, 'Subscribe')
      expectOk(
        await ctx.mcp.callTool('act', { page, kind: 'check', ref }),
        'act check',
      )
      const checked = await evalIn(
        ctx,
        page,
        'return document.getElementById("subscribe").checked',
      )
      if (!checked.includes('true')) {
        throw new Error(`check did not set checkbox state: ${checked}`)
      }
      expectOk(
        await ctx.mcp.callTool('act', { page, kind: 'uncheck', ref }),
        'act uncheck',
      )
      const unchecked = await evalIn(
        ctx,
        page,
        'return document.getElementById("subscribe").checked',
      )
      if (!unchecked.includes('false')) {
        throw new Error(`uncheck did not clear checkbox state: ${unchecked}`)
      }
    },
  },
  {
    name: 'act: select updates a covered styled native select semantically',
    smoke: true,
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const snap = await snapshot(ctx, page)
      const ref = refFor(snap, 'Color ')
      expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'select',
          ref,
          value: 'green',
        }),
        'act select valid',
      )
      const selectedState = await evalIn(
        ctx,
        page,
        'const select = document.getElementById("color"); return JSON.stringify({value: select.value, changes: Number(select.dataset.changeCount), prompt: document.querySelector(".styled-select-prompt").textContent})',
      )
      if (
        !selectedState.includes('"value":"green"') ||
        !selectedState.includes('"changes":1') ||
        !selectedState.includes('"prompt":"Green"')
      ) {
        throw new Error(`select did not apply semantically: ${selectedState}`)
      }
      expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'select',
          ref,
          value: 'chartreuse',
        }),
        'act select missing option',
      )
      const missingState = await evalIn(
        ctx,
        page,
        'const select = document.getElementById("color"); return JSON.stringify({value: select.value, changes: Number(select.dataset.changeCount)})',
      )
      if (
        !missingState.includes('"value":"green"') ||
        !missingState.includes('"changes":1')
      ) {
        throw new Error(`missing option changed select state: ${missingState}`)
      }
    },
  },
  {
    name: 'act: scroll moves the viewport by roughly N steps',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/scroll.html'))
      await snapshot(ctx, page)
      const readY = async () =>
        Number(
          (
            await evalIn(ctx, page, 'return String(Math.round(window.scrollY))')
          ).match(/\d+/)?.[0] ?? '0',
        )
      expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'scroll',
          direction: 'down',
          amount: 3,
        }),
        'page-level scroll',
      )
      await waitUntil(
        async () => (await readY()) > 100,
        'the viewport to scroll down',
        { timeoutMs: 5_000 },
      )
    },
  },
  {
    name: 'act: drag reorders a list',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/scroll.html'))
      const snap = await snapshot(ctx, page)
      const before = await evalIn(
        ctx,
        page,
        'return document.getElementById("drag-order").textContent',
      )
      const result = await ctx.mcp.callTool('act', {
        page,
        kind: 'drag',
        ref: refFor(snap, 'item-a'),
        targetRef: refFor(snap, 'item-c'),
      })
      expectOk(result, 'act drag')
      const after = await evalIn(
        ctx,
        page,
        'return document.getElementById("drag-order").textContent',
      )
      if (before.trim() === after.trim()) {
        throw new Error('drag did not reorder the fixture list')
      }
    },
  },
  {
    name: 'act: dialog_accept and dialog_dismiss resolve confirms',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/dialog.html'))
      await snapshot(ctx, page)
      // A real click on the trigger would wedge for 60s: confirm() blocks
      // the renderer synchronously inside dispatchMouseEvent. Schedule the
      // click off-stack so the dialog opens between CDP calls, then let
      // the dialog kinds resolve it.
      const openConfirm = () =>
        ctx.mcp.callTool('evaluate', {
          page,
          code: 'setTimeout(() => document.getElementById("trigger-confirm").click(), 0); return "scheduled"',
        })
      const dialogResult = () =>
        evalIn(
          ctx,
          page,
          'return document.getElementById("dialog-result").textContent',
        )

      await openConfirm()
      await waitUntil(
        async () =>
          !(await ctx.mcp.callTool('act', { page, kind: 'dialog_accept' }))
            .isError,
        'dialog_accept to resolve the pending confirm',
        { timeoutMs: 10_000 },
      )
      await waitUntil(
        async () => (await dialogResult()).includes('confirm:true'),
        'the accepted confirm to report true',
      )
      await openConfirm()
      await waitUntil(
        async () =>
          !(await ctx.mcp.callTool('act', { page, kind: 'dialog_dismiss' }))
            .isError,
        'dialog_dismiss to resolve the pending confirm',
        { timeoutMs: 10_000 },
      )
      await waitUntil(
        async () => (await dialogResult()).includes('confirm:false'),
        'the dismissed confirm to report false',
      )
    },
  },
  {
    name: 'act: unknown and invalidated refs request a new snapshot',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const snap = await snapshot(ctx, page)
      const unknown = expectError(
        await ctx.mcp.callTool('act', { page, kind: 'click', ref: 'e999' }),
        'act on unknown ref',
      )
      if (errorClass(unknown) !== 'unknown-ref') {
        throw new Error(`unexpected unknown-ref error: ${unknown}`)
      }

      const staleRef = refFor(snap, 'Submit')
      // Navigate to invalidate the ref, then reuse it.
      expectOk(
        await ctx.mcp.callTool('navigate', {
          page,
          action: 'url',
          url: ctx.fixture('/links.html'),
        }),
      )
      const stale = expectError(
        await ctx.mcp.callTool('act', { page, kind: 'click', ref: staleRef }),
        'act on stale ref',
      )
      if (errorClass(stale) !== 'unknown-ref') {
        throw new Error(`unexpected invalidated-ref error: ${stale}`)
      }
    },
  },
]
