import {
  OPENCLAW_CONTAINER_HOME,
  OPENCLAW_TERMINAL_SHELL,
} from '@browseros/shared/constants/openclaw'
import { FitAddon } from '@xterm/addon-fit'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { Terminal } from '@xterm/xterm'
import { ArrowLeft, Check, Copy } from 'lucide-react'
import { type FC, useEffect, useRef, useState } from 'react'
import '@xterm/xterm/css/xterm.css'
import { Button } from '@/components/ui/button'
import { getAgentServerUrl } from '@/lib/browseros/helpers'

interface AgentTerminalProps {
  onBack: () => void
  initialCommand?: string
  onSessionExit?: () => void
}

type TerminalServerMessage =
  | { type: 'output'; data: string }
  | { type: 'exit'; exitCode: number }
  | { type: 'error'; message: string }

const TERMINAL_HOME_DIR = OPENCLAW_CONTAINER_HOME
const TERMINAL_FONT_FAMILY =
  '"Geist Mono", Menlo, Monaco, "Courier New", monospace'

function resolveCssColor(variableName: string): string {
  const probe = document.createElement('div')
  probe.style.position = 'fixed'
  probe.style.visibility = 'hidden'
  probe.style.pointerEvents = 'none'
  probe.style.color = `var(${variableName})`
  document.body.append(probe)
  const color = window.getComputedStyle(probe).color
  probe.remove()
  return color
}

function createTerminalTheme() {
  const isDark = document.documentElement.classList.contains('dark')
  const background = resolveCssColor('--background')
  const foreground = resolveCssColor('--foreground')
  const muted = resolveCssColor('--muted-foreground')

  return {
    background,
    foreground,
    cursor: foreground,
    cursorAccent: background,
    // Solid terminal-standard selection colors. Deriving from a CSS var
    // with alpha composed against the background produced near-white
    // rectangles on light mode, making selection invisible.
    selectionBackground: isDark ? '#3a4463' : '#b4d4f4',
    selectionInactiveBackground: isDark ? '#2b3348' : '#d9e5f3',
    selectionForeground: foreground,
    black: isDark ? '#16131a' : '#1f1b22',
    red: isDark ? '#ef8c7c' : '#c25544',
    green: isDark ? '#9ac67c' : '#5c8754',
    yellow: isDark ? '#e5c07b' : '#b7791f',
    blue: isDark ? '#8ba9ff' : '#4667d8',
    magenta: isDark ? '#d2a8ff' : '#955ec7',
    cyan: isDark ? '#7fd4d1' : '#0f766e',
    white: isDark ? '#e8e0d9' : '#f7f1eb',
    brightBlack: muted,
    brightRed: isDark ? '#ffb0a4' : '#dc7b6d',
    brightGreen: isDark ? '#b6d99e' : '#7bab74',
    brightYellow: isDark ? '#f2d59b' : '#d49a44',
    brightBlue: isDark ? '#b3c4ff' : '#6f8cf0',
    brightMagenta: isDark ? '#e2c6ff' : '#b789dd',
    brightCyan: isDark ? '#a6ece7' : '#3aa5a0',
    brightWhite: isDark ? '#fff8f1' : '#ffffff',
  }
}

function parseTerminalMessage(data: unknown): TerminalServerMessage | null {
  if (typeof data !== 'string') return null

  let parsed: unknown
  try {
    parsed = JSON.parse(data) as unknown
  } catch {
    return null
  }
  if (
    parsed &&
    typeof parsed === 'object' &&
    'type' in parsed &&
    parsed.type === 'output' &&
    'data' in parsed &&
    typeof parsed.data === 'string'
  ) {
    return { type: 'output', data: parsed.data }
  }
  if (
    parsed &&
    typeof parsed === 'object' &&
    'type' in parsed &&
    parsed.type === 'error' &&
    'message' in parsed &&
    typeof parsed.message === 'string'
  ) {
    return { type: 'error', message: parsed.message }
  }
  if (
    parsed &&
    typeof parsed === 'object' &&
    'type' in parsed &&
    parsed.type === 'exit' &&
    'exitCode' in parsed &&
    typeof parsed.exitCode === 'number'
  ) {
    return { type: 'exit', exitCode: parsed.exitCode }
  }
  return null
}

export const AgentTerminal: FC<AgentTerminalProps> = ({
  onBack,
  initialCommand,
  onSessionExit,
}) => {
  const containerRef = useRef<HTMLDivElement>(null)
  const terminalRef = useRef<Terminal | null>(null)
  // Refs keep the mount-once effect from tearing down the PTY when the
  // parent re-renders with new inline callbacks.
  const initialCommandRef = useRef(initialCommand)
  const onSessionExitRef = useRef(onSessionExit)
  initialCommandRef.current = initialCommand
  onSessionExitRef.current = onSessionExit

  const [copied, setCopied] = useState(false)

  // Copy the current xterm selection to the browser clipboard. No-op
  // if nothing is selected — users who want the whole buffer can
  // Cmd+A first. Uses the browser clipboard, not the container's, so
  // it works even when the running TUI has mouse tracking enabled
  // (Opt+drag forces a selection regardless, see terminal config).
  const handleCopy = async (): Promise<void> => {
    const text = terminalRef.current?.getSelection()
    if (!text) return
    try {
      await navigator.clipboard.writeText(text)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1500)
    } catch {
      // clipboard permission denied or unavailable — swallow, user will retry
    }
  }

  useEffect(() => {
    if (!containerRef.current) return

    const terminal = new Terminal({
      fontSize: 14,
      fontFamily: TERMINAL_FONT_FAMILY,
      cursorBlink: true,
      cursorStyle: 'block',
      lineHeight: 1.25,
      scrollback: 8000,
      theme: createTerminalTheme(),
      // Opt+click+drag forces a native text selection even when the
      // running TUI has mouse-tracking enabled (xterm would otherwise
      // forward every click to the app and selection wouldn't work).
      macOptionClickForcesSelection: true,
    })
    terminalRef.current = terminal

    // Cmd+A → select all, Cmd+C → copy selection via the browser
    // clipboard. Return false so xterm doesn't also forward the keys
    // to the running program.
    terminal.attachCustomKeyEventHandler((event) => {
      if (event.type !== 'keydown') return true
      const isMac = navigator.platform.toUpperCase().includes('MAC')
      const mod = isMac ? event.metaKey : event.ctrlKey
      if (!mod) return true
      const key = event.key.toLowerCase()
      if (key === 'a') {
        terminal.selectAll()
        return false
      }
      if (key === 'c') {
        const sel = terminal.getSelection()
        if (sel) {
          void navigator.clipboard.writeText(sel)
          return false
        }
      }
      return true
    })

    const fitAddon = new FitAddon()
    terminal.loadAddon(fitAddon)
    terminal.loadAddon(new WebLinksAddon())
    terminal.open(containerRef.current)

    // React 18 StrictMode double-invokes effects in dev. Everything
    // async inside this effect is scoped to an AbortController; the
    // cleanup aborts it and any pending awaits bail out, so we never
    // leak a second live WebSocket or duplicate xterm listeners.
    const ac = new AbortController()
    const cleanups: Array<() => void> = []
    let ws: WebSocket | null = null
    let sawExit = false

    const applyTheme = (): void => {
      terminal.options.theme = createTerminalTheme()
    }

    const sendMessage = (
      message:
        | { type: 'input'; data: string }
        | { type: 'resize'; cols: number; rows: number },
    ): void => {
      if (ws?.readyState !== WebSocket.OPEN) return
      ws.send(JSON.stringify(message))
    }

    const sendResize = (cols = terminal.cols, rows = terminal.rows): void => {
      sendMessage({ type: 'resize', cols, rows })
    }

    const connect = async (): Promise<void> => {
      const baseUrl = await getAgentServerUrl()
      if (ac.signal.aborted) return
      const wsUrl = new URL('/terminal/ws', baseUrl)
      wsUrl.protocol = wsUrl.protocol === 'https:' ? 'wss:' : 'ws:'

      ws = new WebSocket(wsUrl)
      // If the effect was cleaned up between the await above and now,
      // close the socket we just opened and bail.
      if (ac.signal.aborted) {
        ws.close()
        ws = null
        return
      }
      cleanups.push(() => ws?.close())

      ws.onopen = () => {
        fitAddon.fit()
        terminal.focus()
        sendResize()
        const cmd = initialCommandRef.current
        if (cmd) sendMessage({ type: 'input', data: `${cmd}\n` })
      }

      ws.onmessage = (event) => {
        const message = parseTerminalMessage(event.data)
        if (!message) return

        if (message.type === 'output') {
          terminal.write(message.data)
        } else if (message.type === 'error') {
          terminal.write(`\r\n\x1b[31m${message.message}\x1b[0m\r\n`)
        } else {
          sawExit = true
          terminal.write(
            `\r\n\x1b[90m[session ended with exit ${message.exitCode}]\x1b[0m\r\n`,
          )
          onSessionExitRef.current?.()
        }
      }

      ws.onclose = () => {
        if (sawExit) return
        terminal.write('\r\n\x1b[90m[session ended]\x1b[0m\r\n')
      }

      ws.onerror = () => {
        terminal.write('\r\n\x1b[31m[connection error]\x1b[0m\r\n')
      }

      const inputDisposable = terminal.onData((data) => {
        sendMessage({ type: 'input', data })
      })
      const resizeDisposable = terminal.onResize(({ cols, rows }) => {
        sendResize(cols, rows)
      })
      cleanups.push(() => inputDisposable.dispose())
      cleanups.push(() => resizeDisposable.dispose())
    }

    void connect()

    const resizeObserver = new ResizeObserver(() => {
      fitAddon.fit()
      sendResize()
    })
    resizeObserver.observe(containerRef.current)
    cleanups.push(() => resizeObserver.disconnect())

    const themeObserver = new MutationObserver(() => applyTheme())
    themeObserver.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ['class'],
    })
    cleanups.push(() => themeObserver.disconnect())

    return () => {
      ac.abort()
      for (const dispose of cleanups) dispose()
      terminal.dispose()
      terminalRef.current = null
    }
  }, [])

  return (
    <div className="flex h-[calc(100dvh-10rem)] min-h-[32rem] w-full flex-col py-2 sm:min-h-[42rem] sm:py-4">
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-xl border border-border bg-card shadow-sm">
        <div className="flex items-center justify-between gap-3 border-border border-b px-4 py-3 sm:px-6">
          <div className="flex min-w-0 items-center gap-3">
            <Button variant="ghost" size="icon" onClick={onBack}>
              <ArrowLeft className="size-4" />
            </Button>
            <div className="min-w-0">
              <div className="truncate font-semibold text-sm">
                Container Terminal
              </div>
              <div className="truncate text-muted-foreground text-sm">
                OpenClaw shell in {TERMINAL_HOME_DIR}
              </div>
            </div>
          </div>
          <Button variant="outline" size="sm" onClick={handleCopy}>
            {copied ? (
              <Check className="mr-1 size-3.5" />
            ) : (
              <Copy className="mr-1 size-3.5" />
            )}
            {copied ? 'Copied' : 'Copy'}
          </Button>
        </div>

        <div className="min-h-0 flex-1 p-4 sm:p-6">
          <div className="agent-terminal-shell flex h-full min-h-0 flex-col overflow-hidden rounded-lg border border-border bg-background">
            <div className="flex items-center justify-between gap-3 border-border border-b px-4 py-2.5">
              <div className="truncate font-mono text-muted-foreground text-xs">
                {TERMINAL_HOME_DIR}
              </div>
              <div className="font-mono text-[11px] text-muted-foreground">
                {OPENCLAW_TERMINAL_SHELL.split('/').pop()}
              </div>
            </div>

            <div className="min-h-0 flex-1 cursor-text px-4 py-4 sm:px-5 sm:py-5">
              <div ref={containerRef} className="h-full w-full" />
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
