import { ArrowUpRight } from 'lucide-react'
import { ClaudeCodeMark } from '@/components/harness/harness-marks'

const EXTENSION_INSTALL_URL =
  'https://github.com/browseros-ai/browserclaw-claude-desktop#install-the-extension'

/**
 * Advertises the BrowserClaw extension for Claude Desktop. Unlike the
 * harness rows, Claude Desktop connects by dragging a `.mcpb` into its
 * Settings, which this app cannot toggle or detect, so this is a link out
 * to the repo install steps rather than a connect action.
 */
export function ClaudeDesktopCard() {
  return (
    <section className="space-y-2">
      <header className="flex items-baseline justify-between gap-3">
        <h2 className="font-semibold text-ink text-lg">Claude Desktop</h2>
        <span className="font-mono text-[10.5px] text-ink-3 uppercase tracking-[0.08em]">
          browser extension
        </span>
      </header>
      <a
        href={EXTENSION_INSTALL_URL}
        target="_blank"
        rel="noreferrer"
        className="group block rounded-xl border border-border-2 bg-card-tint px-4 py-4 transition-colors hover:border-accent/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/40"
      >
        <div className="flex items-start gap-3">
          <ClaudeCodeMark className="size-7 shrink-0" />
          <div className="min-w-0 flex-1 space-y-1">
            <p className="font-semibold text-[15px] text-ink leading-snug">
              Give Claude Desktop a real browser.
            </p>
            <p className="text-[13px] text-ink-2 leading-snug">
              Drop in the extension and Claude reaches for BrowserClaw to open
              sites, log in, and click through flows.
            </p>
            <p className="text-[12px] text-ink-3 leading-snug">
              Also shipping as Cowork.
            </p>
          </div>
        </div>
        <div className="mt-3 flex justify-end">
          <span className="inline-flex items-center gap-1 font-mono text-[11px] text-accent uppercase tracking-[0.08em] transition-colors group-hover:text-accent-2">
            install the extension
            <ArrowUpRight
              aria-hidden
              className="size-3.5 transition-transform group-hover:translate-x-0.5 group-hover:-translate-y-0.5"
            />
          </span>
        </div>
      </a>
    </section>
  )
}
