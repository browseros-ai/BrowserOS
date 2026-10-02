/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The in-page takeover bar: a shadow-DOM control the human uses to hand control
 * back to a parked agent without leaving the blocked page. Built with plain DOM
 * (no framework) so the content script stays small. Leads with BrowserOS neo
 * branding (the dog mark and Signal Blue) because it rides a third-party page
 * where provenance matters; the needs-you urgency is the secondary amber accent.
 */

import { NEO_MARK_SVG } from './neo-mark'
import type { TakeoverContext } from './takeover.types'

export interface TakeoverBarHandlers {
  onHandBack: (note: string) => void
}

export interface TakeoverBar {
  /** Show the bar for this context, or hide it when null. */
  update: (context: TakeoverContext | null) => void
  destroy: () => void
}

// Signal Blue (--accent) leads; burnt orange (#b85c10) carries needs-you. The
// shadow root resets inherited styles, so fonts and colors are set explicitly.
export const TAKEOVER_BAR_CSS = `
:host { all: initial; }
.nt-root { position: fixed; left: 16px; right: 16px; bottom: 16px; z-index: 2147483000;
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif; color: #0b1b33; }
.nt-bar { background: #fff; border: 1px solid rgba(2,84,236,0.18); border-top: 3px solid #0254ec;
  border-radius: 12px; box-shadow: 0 10px 30px rgba(2,22,51,0.30); overflow: hidden; max-width: 760px; margin: 0 auto; }
.nt-head { display: flex; align-items: center; gap: 10px; padding: 9px 12px; background: #eaf1ff; border-bottom: 1px solid rgba(2,84,236,0.18); }
.nt-dog { width: 22px; height: 22px; flex: 0 0 auto; display: block; }
.nt-dog svg { width: 22px; height: 22px; border-radius: 6px; display: block; }
.nt-brand { font-weight: 800; font-size: 12.5px; letter-spacing: -0.01em; color: #0254ec; }
.nt-spacer { flex: 1 1 auto; }
.nt-pill { display: inline-flex; align-items: center; gap: 6px; background: #b85c10; color: #fff;
  border-radius: 999px; padding: 3px 9px; font-size: 10.5px; font-weight: 700; letter-spacing: 0.06em; text-transform: uppercase; }
.nt-pill .nt-timer { font-weight: 600; letter-spacing: 0; font-variant-numeric: tabular-nums; }
.nt-min { width: 26px; height: 26px; border: 1px solid rgba(2,84,236,0.18); border-radius: 7px; background: #fff;
  color: #6b7b85; cursor: pointer; font-size: 15px; line-height: 1; display: flex; align-items: center; justify-content: center; padding: 0; }
.nt-min:hover { background: #f2f5fb; }
.nt-body { padding: 12px 14px 14px; }
.nt-title { font-weight: 700; font-size: 14px; }
.nt-title .nt-agent { color: #0b1b33; }
.nt-title .nt-site { color: #0254ec; }
.nt-reason { font-size: 13px; color: #41525c; margin-top: 3px; }
.nt-resume { font-size: 12.5px; color: #6b7b85; margin-top: 2px; }
.nt-actions { display: flex; gap: 10px; align-items: center; margin-top: 12px; flex-wrap: wrap; }
.nt-note { flex: 1 1 200px; min-width: 150px; height: 36px; border: 1px solid rgba(2,84,236,0.18); border-radius: 8px;
  padding: 0 10px; font: inherit; font-size: 13px; color: #0b1b33; background: #fff; }
.nt-note::placeholder { color: #6b7b85; }
.nt-handback { height: 36px; padding: 0 16px; border: 0; border-radius: 8px; background: #0254ec; color: #fff;
  font: inherit; font-weight: 600; font-size: 13px; cursor: pointer; }
.nt-handback:hover { background: #0246c6; }
.nt-handback:disabled { opacity: 0.6; cursor: default; }
.nt-pill-btn { display: inline-flex; align-items: center; gap: 9px; background: #0254ec; color: #fff; border: 0;
  border-radius: 999px; padding: 7px 13px 7px 7px; box-shadow: 0 8px 22px rgba(2,22,51,0.4); font: inherit;
  font-size: 13px; font-weight: 600; cursor: pointer; float: right; }
.nt-chip { width: 24px; height: 24px; border-radius: 7px; background: #fff; display: flex; align-items: center; justify-content: center; }
.nt-chip svg { width: 18px; height: 18px; border-radius: 4px; display: block; }
.nt-dot { width: 7px; height: 7px; border-radius: 50%; background: #f6b53d; box-shadow: 0 0 0 2px rgba(246,181,61,0.3); }
.nt-pill-btn .nt-timer { font-variant-numeric: tabular-nums; opacity: 0.92; }
.nt-chev { opacity: 0.9; }
`

function waitLabel(requestedAt: number, now: number): string {
  const seconds = Math.max(0, Math.floor((now - requestedAt) / 1000))
  const minutes = Math.floor(seconds / 60)
  return `${minutes}:${String(seconds % 60).padStart(2, '0')}`
}

export function createTakeoverBar(
  root: HTMLElement,
  handlers: TakeoverBarHandlers,
): TakeoverBar {
  const doc = root.ownerDocument
  const el = <K extends keyof HTMLElementTagNameMap>(
    tag: K,
    className?: string,
  ): HTMLElementTagNameMap[K] => {
    const node = doc.createElement(tag)
    if (className) node.className = className
    return node
  }

  const wrap = el('div', 'nt-root')
  wrap.hidden = true

  // Expanded bar.
  const bar = el('div', 'nt-bar')
  const head = el('div', 'nt-head')
  const dog = el('span', 'nt-dog')
  dog.innerHTML = NEO_MARK_SVG
  const brand = el('span', 'nt-brand')
  brand.textContent = 'BrowserOS neo'
  const spacer = el('span', 'nt-spacer')
  const pill = el('span', 'nt-pill')
  pill.append('Needs you ')
  const timer = el('span', 'nt-timer')
  pill.append(timer)
  const minimize = el('button', 'nt-min')
  minimize.type = 'button'
  minimize.title = 'Minimize'
  minimize.setAttribute('aria-label', 'Minimize')
  minimize.textContent = '−'
  head.append(dog, brand, spacer, pill, minimize)

  const body = el('div', 'nt-body')
  const title = el('div', 'nt-title')
  title.append("You're in control of ")
  const agent = el('b', 'nt-agent')
  const site = el('span', 'nt-site')
  title.append(agent, ' ', site)
  const reason = el('div', 'nt-reason')
  const resume = el('div', 'nt-resume')
  const actions = el('div', 'nt-actions')
  const note = el('input', 'nt-note')
  note.type = 'text'
  note.placeholder = 'note for the agent (optional)'
  note.setAttribute('aria-label', 'Note for the agent')
  const handback = el('button', 'nt-handback')
  handback.type = 'button'
  handback.textContent = 'Hand back to the agent'
  actions.append(note, handback)
  body.append(title, reason, resume, actions)
  bar.append(head, body)

  // Minimized pill.
  const pillBtn = el('button', 'nt-pill-btn')
  pillBtn.type = 'button'
  pillBtn.setAttribute('aria-label', 'Expand BrowserOS neo takeover controls')
  const chip = el('span', 'nt-chip')
  chip.innerHTML = NEO_MARK_SVG
  const pillAgent = el('span')
  const dot = el('span', 'nt-dot')
  const pillTimer = el('span', 'nt-timer')
  const chev = el('span', 'nt-chev')
  chev.textContent = '›'
  pillBtn.append(chip, pillAgent, ' needs you ', dot, pillTimer, chev)
  pillBtn.hidden = true

  wrap.append(bar, pillBtn)
  root.append(wrap)

  let current: TakeoverContext | null = null
  let minimized = false

  const renderTimer = () => {
    if (!current) return
    const label = waitLabel(current.requestedAt, Date.now())
    timer.textContent = label
    pillTimer.textContent = label
  }

  const render = () => {
    if (!current) {
      wrap.hidden = true
      return
    }
    wrap.hidden = false
    agent.textContent = current.agentLabel
    site.textContent = current.site ? `on ${current.site}` : ''
    reason.textContent = current.reason
    resume.hidden = !current.resumeHint
    resume.textContent = current.resumeHint
      ? `When done, the agent resumes at: ${current.resumeHint}`
      : ''
    pillAgent.textContent = current.agentLabel
    renderTimer()
    bar.hidden = minimized
    pillBtn.hidden = !minimized
  }

  minimize.addEventListener('click', () => {
    minimized = true
    render()
  })
  pillBtn.addEventListener('click', () => {
    minimized = false
    render()
  })
  handback.addEventListener('click', () => {
    handback.disabled = true
    handlers.onHandBack(note.value.trim())
  })

  // A light tick keeps the timer moving between the content script's polls.
  const ticker =
    typeof setInterval === 'function'
      ? setInterval(renderTimer, 1000)
      : undefined

  return {
    update(context) {
      const sameRequest =
        context != null &&
        current != null &&
        context.sessionId === current.sessionId &&
        context.requestedAt === current.requestedAt
      current = context
      // A new request (or reappearance) resets the per-request UI state.
      if (!sameRequest) {
        minimized = false
        note.value = ''
        handback.disabled = false
      }
      render()
    },
    destroy() {
      if (ticker !== undefined) clearInterval(ticker)
      wrap.remove()
    },
  }
}
