import { ArrowDownToLine, Cpu, ShieldCheck } from 'lucide-react'

/** Persistent value rail beside the onboarding steps, in the app theme. */
export function VisualRail() {
  return (
    <div
      className="relative flex w-[360px] shrink-0 flex-col justify-between overflow-hidden border-border border-r p-9"
      style={{
        background:
          'linear-gradient(165deg, var(--color-accent-tint) 0%, var(--color-bg-sunken) 55%, var(--color-bg-canvas) 100%)',
      }}
    >
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0"
        style={{
          background:
            'radial-gradient(420px 300px at 30% 12%, var(--color-accent-tint-2), transparent 70%)',
        }}
      />
      <div className="relative flex items-center gap-2.5">
        {/* Must stay a literal public path: importing the asset or inlining it
            breaks scripts/verify-chromium-build.ts (allowlist, no data: URLs). */}
        <img
          alt=""
          aria-hidden
          className="size-[38px] rounded-[11px]"
          src="/icon/128.png"
        />
        <div className="font-extrabold text-[17px] tracking-tight">
          BrowserOS
        </div>
      </div>
      <div className="relative">
        <div className="mb-[22px] font-semibold text-[21px] text-ink leading-snug">
          The Open Source <span className="text-accent">agentic browser</span>{' '}
          to automate any web task
        </div>
        <div className="flex flex-col gap-3">
          {FEATURES.map((feature) => {
            const Icon = feature.icon
            return (
              <div key={feature.title} className="flex items-start gap-[11px]">
                <span className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-card/70 text-accent-ink">
                  <Icon className="size-[15px]" />
                </span>
                <div>
                  <div className="font-bold text-[13.5px]">{feature.title}</div>
                  <div className="text-[12px] text-ink-2">
                    {feature.description}
                  </div>
                </div>
              </div>
            )
          })}
        </div>
      </div>
      <div className="relative text-[11.5px] text-ink-3">
        Local-first. Private.
      </div>
    </div>
  )
}

const FEATURES = [
  {
    icon: ArrowDownToLine,
    title: 'Bring your browser',
    description: 'History, bookmarks, and saved logins.',
  },
  {
    icon: Cpu,
    title: 'Your agent, your way',
    description: 'Any LLM provider or coding agent.',
  },
  {
    icon: ShieldCheck,
    title: 'Runs on your machine',
    description: 'Local-first and private.',
  },
] as const
