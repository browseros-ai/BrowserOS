/**
 * act (20) — every kind and every failure path. Snapshots are taken
 * before each action to mint fresh refs; DOM state is verified through
 * evaluate (`return …`, since evaluate runs the code as a function body).
 */

import type { CaseContext, ContractCase } from './cases'
import { expectError, expectOk, waitUntil } from './helpers'
import { textOf } from './mcp-client'
import { errorClass } from './parity'

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
      ctx.record('act:click-updates-result', true)
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
      // Either the click is refused naming the overlay, or it lands but
      // the overlay intercepts it (result stays untouched). Record which.
      const text = textOf(result)
      const namesBlocker = /overlay|cover|intercept|obscur/i.test(text)
      const stillUntouched = (
        await evalIn(
          ctx,
          page,
          'return document.getElementById("overlay-result").textContent',
        )
      ).includes('untouched')
      if (!namesBlocker && !stillUntouched) {
        throw new Error(
          `covered click neither blocked nor named a blocker: ${text}`,
        )
      }
      // Shared contract: the covered button is never activated. Rust names
      // the intercepting overlay in its error; TS does not (divergence
      // covered-click-blocker-naming).
      ctx.record('act:covered-click-not-activated', stillUntouched)
      ctx.record('act:covered-click-names-blocker', namesBlocker, {
        divergence: 'covered-click-blocker-naming',
      })
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
      ctx.record('act:click-at-hits-coords', true)
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
      ctx.record('act:type-enters-text', true)
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
      ctx.record('act:type-at-supported', !result.isError)
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
      ctx.record('act:fill-single', true)
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
      ctx.record('act:fill-batch', true)
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
      ctx.record('act:press-enter-submits', true)
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
      ctx.record('act:shift-a-uppercase', true)
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
      ctx.record('act:cmd-c-alias-resolves', true)
    },
  },
  {
    name: 'act: press with an invalid key errors and lists keys',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const snap = await snapshot(ctx, page)
      const text = expectError(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'press',
          ref: refFor(snap, '"Name '),
          key: 'NotARealKey',
        }),
        'act press invalid key',
      )
      ctx.record('act:invalid-key-errors', text.length > 0)
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
      ctx.record('act:hover-reveals-tooltip', visible.includes('block'))
    },
  },
  {
    name: 'act: focus by ref (shared limitation: errors on both servers)',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const snap = await snapshot(ctx, page)
      // VERIFIED SHARED BUG: `act kind=focus` by ref fails identically on
      // both servers with `CDP error: Document needs to be requested
      // first` — focusElement() calls DOM.pushNodesByBackendIdsToFrontend
      // without a prior DOM.getDocument, which a snapshot (Accessibility
      // domain) never primes. Parity holds; a real click is the working
      // focus path. Recorded so a one-sided fix trips the parity gate.
      const result = await ctx.mcp.callTool('act', {
        page,
        kind: 'focus',
        ref: refFor(snap, '"Bio '),
      })
      const active = await evalIn(
        ctx,
        page,
        'return document.activeElement && document.activeElement.id',
      )
      ctx.record('act:focus-by-ref', {
        errored: result.isError === true,
        movedActiveElement: active.includes('bio'),
      })
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
      ctx.record('act:checkbox-toggle-repeatable', true)
    },
  },
  {
    name: 'act: check and uncheck kinds set checkbox state',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const snap = await snapshot(ctx, page)
      const ref = refFor(snap, 'Subscribe')
      const checked = await ctx.mcp.callTool('act', {
        page,
        kind: 'check',
        ref,
      })
      const unchecked = await ctx.mcp.callTool('act', {
        page,
        kind: 'uncheck',
        ref,
      })
      // Rust's check/uncheck kinds are broken (divergence act-check-kind);
      // TS applies them.
      ctx.record(
        'act:check-uncheck-kinds',
        { checkOk: !checked.isError, uncheckOk: !unchecked.isError },
        { divergence: 'act-check-kind' },
      )
      if (ctx.server.name === 'typescript') {
        if (checked.isError || unchecked.isError) {
          throw new Error('check/uncheck failed on typescript')
        }
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
      const invalid = await ctx.mcp.callTool('act', {
        page,
        kind: 'select',
        ref,
        value: 'chartreuse',
      })
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
      ctx.record('act:styled-select-semantics', {
        validApplied: true,
        changeBubbledOnce: true,
        missingLeftStateUnchanged: true,
        invalidRejected: invalid.isError === true,
      })
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
      const scroll = await ctx.mcp.callTool('act', {
        page,
        kind: 'scroll',
        direction: 'down',
        amount: 3,
      })
      // TS scrolls ~amount*120px (wheel scroll can settle async); rust's
      // page scroll is broken today — Input.dispatchMouseEvent times out
      // (divergence act-scroll). Record actual behavior; assert only on TS.
      let y = 0
      if (!scroll.isError) {
        await waitUntil(
          async () => {
            y = await readY()
            return y > 100
          },
          'the viewport to scroll down',
          { timeoutMs: 5_000 },
        ).catch(() => {})
      }
      ctx.record(
        'act:scroll-moves-viewport',
        { scrolled: y > 100, errored: scroll.isError === true },
        { divergence: 'act-scroll' },
      )
      if (ctx.server.name === 'typescript' && y <= 100) {
        throw new Error(
          `scroll did not move the viewport on typescript: y=${y}`,
        )
      }
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
      const after = await evalIn(
        ctx,
        page,
        'return document.getElementById("drag-order").textContent',
      )
      ctx.record('act:drag-reorders', {
        accepted: result.isError !== true,
        orderChanged: before.trim() !== after.trim(),
      })
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
      // the dialog kinds resolve it. TS has no dialog kinds (divergence
      // act-dialog-kinds), so only rust runs the full flow.
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

      if (ctx.server.name === 'rust') {
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
        ctx.record('act:dialog-kinds-supported', true, {
          divergence: 'act-dialog-kinds',
        })
      } else {
        // No dialog opened on TS; the kind itself is rejected.
        const accept = await ctx.mcp.callTool('act', {
          page,
          kind: 'dialog_accept',
        })
        ctx.record('act:dialog-kinds-supported', !accept.isError, {
          divergence: 'act-dialog-kinds',
        })
      }
    },
  },
  {
    name: 'act: unknown and stale refs return take-a-new-snapshot errors',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const snap = await snapshot(ctx, page)
      const unknown = expectError(
        await ctx.mcp.callTool('act', { page, kind: 'click', ref: 'e999' }),
        'act on unknown ref',
      )
      ctx.record('act:unknown-ref-class', errorClass(unknown))

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
      ctx.record('act:stale-ref-class', errorClass(stale))
    },
  },
]
