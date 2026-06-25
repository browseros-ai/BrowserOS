import { Bolt, Lock, ShieldCheck, Sparkles } from 'lucide-react'
import { type ComponentType, type SVGProps, useState } from 'react'
import { useNavigate } from 'react-router'
import { cn } from '@/lib/utils'
import { ConnectStep } from './ConnectStep'
import { ImportLoginsStep } from './ImportLoginsStep'
import {
  DEFAULT_PROFILE_IDS,
  ONBOARDING_STEP_COUNT,
} from './onboarding.helpers'
import { ReadyStep } from './ReadyStep'
import { WelcomeStep } from './WelcomeStep'

export function Onboarding() {
  const navigate = useNavigate()
  const [step, setStep] = useState(0)
  const [pickedIds, setPickedIds] = useState<string[]>([...DEFAULT_PROFILE_IDS])

  const close = () => navigate('/')

  return (
    <div className="flex h-screen min-h-0 items-center justify-center bg-bg-canvas p-4">
      <div className="flex h-full max-h-[720px] w-full max-w-[1040px] overflow-hidden rounded-2xl border border-border bg-card shadow-2xl">
        <BrandColumn />
        <div className="flex flex-1 flex-col overflow-y-auto px-12 py-11">
          <div className="mb-7">
            <StepDots total={ONBOARDING_STEP_COUNT} active={step} />
          </div>
          {step === 0 && (
            <WelcomeStep onSetup={() => setStep(1)} onSkip={close} />
          )}
          {step === 1 && (
            <ImportLoginsStep
              pickedIds={pickedIds}
              onPickedChange={setPickedIds}
              onContinue={() => setStep(2)}
            />
          )}
          {step === 2 && <ConnectStep onContinue={() => setStep(3)} />}
          {step === 3 && <ReadyStep onDone={close} />}
        </div>
      </div>
    </div>
  )
}

/* ---------------------------------------------------------------------------
 * Sub-components, kept private to the onboarding shell.
 * -------------------------------------------------------------------------*/

interface ValueProp {
  Icon: ComponentType<SVGProps<SVGSVGElement>>
  title: string
  detail: string
}

const VALUE_PROPS: readonly ValueProp[] = [
  {
    Icon: Bolt,
    title: 'Fast & token-cheap',
    detail: 'DOM-first, not a screenshot loop',
  },
  {
    Icon: Lock,
    title: 'Logged in as you',
    detail: 'Imports your Chrome sessions',
  },
  {
    Icon: ShieldCheck,
    title: 'Under your control',
    detail: 'Scoped approvals, hard blocks',
  },
]

function BrandColumn() {
  return (
    <aside className="relative flex w-[360px] shrink-0 flex-col justify-between overflow-hidden border-border border-r bg-gradient-to-br from-[#F8DCC2] via-[#FBF0E4] to-[#FAF8F5] p-9">
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 bg-[radial-gradient(420px_300px_at_30%_12%,rgba(242,107,42,0.16),transparent_70%)]"
      />
      <div className="relative flex items-center gap-2.5">
        <span className="flex size-9 items-center justify-center rounded-xl bg-accent text-white">
          <Sparkles className="size-5" />
        </span>
        <div className="font-extrabold text-[17px] text-ink tracking-tight">
          BrowserOS
        </div>
      </div>
      <div className="relative">
        <blockquote className="mb-4 font-['Newsreader',serif] text-[21px] text-ink italic leading-snug">
          "Let the agent you already run drive the browser you're already logged
          into."
        </blockquote>
        <ul className="flex flex-col gap-3">
          {VALUE_PROPS.map(({ Icon, title, detail }) => (
            <li key={title} className="flex items-start gap-3">
              <span className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-white/70 text-accent-ink">
                <Icon className="size-3.5" />
              </span>
              <div>
                <div className="font-bold text-ink text-sm">{title}</div>
                <div className="text-ink-2 text-xs">{detail}</div>
              </div>
            </li>
          ))}
        </ul>
      </div>
      <div className="relative text-ink-3 text-xs">
        Mac · v1.0 · signed build
      </div>
    </aside>
  )
}

interface StepDotsProps {
  total: number
  active: number
}

function StepDots({ total, active }: StepDotsProps) {
  return (
    <div className="flex items-center gap-2">
      {Array.from({ length: total }, (_, i) => i).map((i) => (
        <span
          key={`dot-${i}`}
          aria-current={i === active ? 'step' : undefined}
          className={cn(
            'h-1.5 rounded-full transition-all duration-300',
            i === active
              ? 'w-[22px] bg-accent'
              : i < active
                ? 'w-1.5 bg-accent-tint-2'
                : 'w-1.5 bg-border-2',
          )}
        />
      ))}
    </div>
  )
}
