import {
  AlertTriangle,
  Check,
  ChevronRight,
  CreditCard,
  Download,
  Loader2,
  Lock,
  User,
} from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import {
  CHROME_PROFILES,
  type ImportResult,
  useImportChromeSessions,
} from '@/modules/api/onboarding.hooks'
import { summariseProfileSelection } from './onboarding.helpers'

interface ImportLoginsStepProps {
  pickedIds: string[]
  onPickedChange: (next: string[]) => void
  onContinue: (result: ImportResult) => void
}

type Stage = 'chrome-open' | 'pick' | 'importing' | 'imported'

/**
 * Step 1. Walks the user through quitting Chrome, picking which
 * profiles to import, watching the progress, and seeing a summary
 * before moving on.
 */
export function ImportLoginsStep({
  pickedIds,
  onPickedChange,
  onContinue,
}: ImportLoginsStepProps) {
  const [stage, setStage] = useState<Stage>('chrome-open')
  const [result, setResult] = useState<ImportResult | null>(null)
  const { selected, totalSites, totalLogins } =
    summariseProfileSelection(pickedIds)
  const importSessions = useImportChromeSessions()

  const togglePicked = (id: string) => {
    onPickedChange(
      pickedIds.includes(id)
        ? pickedIds.filter((p) => p !== id)
        : [...pickedIds, id],
    )
  }

  const startImport = () => {
    if (selected.length === 0) return
    setStage('importing')
    importSessions.mutate(
      { profileIds: pickedIds },
      {
        onSuccess: (data) => {
          setResult(data)
          setStage('imported')
        },
      },
    )
  }

  return (
    <div className="flex w-full max-w-[560px] flex-col">
      <Headline>
        Import your{' '}
        <em className="font-['Newsreader',serif] font-medium text-accent italic">
          logins
        </em>
        .
      </Headline>
      <p className="mb-5 max-w-[470px] text-ink-2 text-sm leading-relaxed">
        BrowserOS copies your saved Chrome sessions so the agent never has to
        log in again. Sessions stay in a local vault on this Mac.
      </p>

      {stage === 'chrome-open' && (
        <ChromeQuitPrompt onConfirm={() => setStage('pick')} />
      )}

      {stage === 'pick' && (
        <>
          <h2 className="mb-2.5 font-bold text-ink-2 text-xs">
            Choose which Chrome profiles to import
          </h2>
          <div className="mb-4 flex flex-col gap-2">
            {CHROME_PROFILES.map((profile) => {
              const on = pickedIds.includes(profile.id)
              return (
                <button
                  key={profile.id}
                  type="button"
                  onClick={() => togglePicked(profile.id)}
                  className={cn(
                    'flex items-center gap-3 rounded-xl border p-3 text-left transition-colors',
                    on
                      ? 'border-accent bg-accent-tint'
                      : 'border-border-2 bg-card hover:border-border-strong',
                  )}
                >
                  <span
                    className={cn(
                      'flex size-5 shrink-0 items-center justify-center rounded-md border-[1.5px]',
                      on
                        ? 'border-accent bg-accent text-card'
                        : 'border-border-strong bg-card',
                    )}
                  >
                    {on && <Check className="size-3" />}
                  </span>
                  <span className="flex size-8 shrink-0 items-center justify-center rounded-lg border border-border-2 bg-card text-ink-2">
                    <User className="size-3.5" />
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="truncate font-bold text-ink text-sm">
                      {profile.name}
                    </div>
                    <div className="truncate text-ink-3 text-xs">
                      {profile.email}
                    </div>
                  </div>
                  <div className="shrink-0 text-right font-mono text-ink-2 text-xs">
                    {profile.sites} sites · {profile.logins} logins
                  </div>
                </button>
              )
            })}
          </div>
          <KeychainNotice />
          <Button
            type="button"
            size="lg"
            onClick={startImport}
            disabled={selected.length === 0}
            className="self-start"
          >
            <Download className="size-4" />
            {selected.length === 0
              ? 'Pick at least one profile'
              : `Import ${totalSites} sites from ${selected.length} profile${selected.length > 1 ? 's' : ''}`}
          </Button>
        </>
      )}

      {stage === 'importing' && (
        <div className="rounded-2xl border border-border-2 bg-card p-5">
          <div className="mb-3 flex items-center gap-2.5">
            <Loader2 className="size-4 animate-spin text-accent" />
            <span className="font-bold text-ink text-sm">
              Importing sessions…
            </span>
          </div>
          <p className="font-mono text-ink-2 text-xs">
            {totalSites} sites · {totalLogins} passwords
          </p>
        </div>
      )}

      {stage === 'imported' && result && (
        <div className="flex flex-col gap-4">
          <div className="rounded-2xl border border-border-2 bg-card p-5">
            <div className="mb-3 flex items-center gap-2.5">
              <span className="flex size-7 items-center justify-center rounded-lg bg-green-tint text-green">
                <Check className="size-4" />
              </span>
              <span className="font-bold text-ink text-sm">
                Imported {result.importedSites} sites from {selected.length}{' '}
                profile
                {selected.length > 1 ? 's' : ''}
              </span>
            </div>
            <ImportSummaryLine
              icon={<Check className="size-3.5 text-green" />}
              text={`${result.importedSites} logged-in sessions ready`}
            />
            <ImportSummaryLine
              icon={<Lock className="size-3.5 text-ink-3" />}
              text="Passwords stored in vault, never shown to you or the agent"
            />
            <ImportSummaryLine
              icon={<CreditCard className="size-3.5 text-ink-3" />}
              text="3 payment cards skipped"
            />
          </div>
          <Button
            type="button"
            size="lg"
            onClick={() => onContinue(result)}
            className="self-start"
          >
            <ChevronRight className="size-4" />
            Connect to Claude
          </Button>
        </div>
      )}
    </div>
  )
}

/* ---------------------------------------------------------------------------
 * Sub-components, private to this step.
 * -------------------------------------------------------------------------*/

function Headline({ children }: { children: React.ReactNode }) {
  return (
    <h1 className="mb-3 font-extrabold text-[34px] text-ink leading-[1.05] tracking-tight">
      {children}
    </h1>
  )
}

function ChromeQuitPrompt({ onConfirm }: { onConfirm: () => void }) {
  return (
    <div className="mb-4 flex gap-3 rounded-xl border border-[#ECD9AC] bg-amber-tint p-4">
      <AlertTriangle className="mt-0.5 size-4 shrink-0 text-amber" />
      <div className="flex-1">
        <div className="mb-1 font-bold text-ink text-sm">Chrome is open</div>
        <p className="mb-3 text-ink-2 text-xs leading-snug">
          It needs to close so we can read your data safely. We'll never
          force-quit or touch your profile.
        </p>
        <Button type="button" size="sm" variant="outline" onClick={onConfirm}>
          Quit Chrome for me
        </Button>
      </div>
    </div>
  )
}

function KeychainNotice() {
  return (
    <div className="mb-4 flex gap-3 rounded-xl border border-[#D3E0F0] bg-[#EEF3FA] p-4">
      <Lock className="mt-0.5 size-4 shrink-0 text-blue" />
      <p className="text-ink-2 text-xs leading-snug">
        <strong className="font-semibold text-ink">
          macOS will ask permission
        </strong>{' '}
        to read Chrome's saved data. That's expected, click{' '}
        <strong className="font-semibold text-ink">Allow</strong> on the
        Keychain prompt.
      </p>
    </div>
  )
}

function ImportSummaryLine({
  icon,
  text,
}: {
  icon: React.ReactNode
  text: string
}) {
  return (
    <div className="flex items-center gap-2.5 py-1 text-ink-2 text-xs">
      {icon}
      {text}
    </div>
  )
}
