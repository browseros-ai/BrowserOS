import { Lock, ShieldCheck, Zap } from 'lucide-react'

/**
 * Left visual rail of the onboarding shell. 360 px wide with a
 * warm-orange linear gradient + radial wash, a BrowserOS logo + word
 * mark at the top, an italic serif quote in the middle, three small
 * feature blocks below it, and a faint footer caption. Lifted
 * verbatim from the v2 design's left column.
 */
export function VisualRail() {
  return (
    <div
      className="relative flex w-[360px] shrink-0 flex-col justify-between overflow-hidden border-border border-r p-9"
      style={{
        background:
          'linear-gradient(165deg, #F8DCC2 0%, #FBF0E4 55%, #FAF8F5 100%)',
      }}
    >
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0"
        style={{
          background:
            'radial-gradient(420px 300px at 30% 12%, rgba(242,107,42,.16), transparent 70%)',
        }}
      />
      <div className="relative flex items-center gap-2.5">
        <div className="flex size-[38px] items-center justify-center rounded-[11px] bg-accent font-extrabold text-card text-lg">
          B
        </div>
        <div className="font-extrabold text-[17px] tracking-tight">
          BrowserOS
        </div>
      </div>
      <div className="relative">
        <div className="mb-[18px] font-serif text-[23px] text-ink italic leading-snug">
          &ldquo;Let the agent you already run drive the browser you&rsquo;re
          already logged into.&rdquo;
        </div>
        <div className="flex flex-col gap-3">
          {FEATURES.map((f) => {
            const Icon = f.icon
            return (
              <div key={f.title} className="flex items-start gap-[11px]">
                <span className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-white/70 text-accent-ink">
                  <Icon className="size-[15px]" />
                </span>
                <div>
                  <div className="font-bold text-[13.5px]">{f.title}</div>
                  <div className="text-[12px] text-ink-2">{f.description}</div>
                </div>
              </div>
            )
          })}
        </div>
      </div>
      <div className="relative text-[11.5px] text-ink-3">
        Mac . v1.0 . signed build
      </div>
    </div>
  )
}

const FEATURES = [
  {
    icon: Zap,
    title: 'Fast & token-cheap',
    description: 'DOM-first, not a screenshot loop',
  },
  {
    icon: Lock,
    title: 'Logged in as you',
    description: 'Imports your Chrome sessions',
  },
  {
    icon: ShieldCheck,
    title: 'Under your control',
    description: 'Scoped approvals, hard blocks',
  },
] as const
