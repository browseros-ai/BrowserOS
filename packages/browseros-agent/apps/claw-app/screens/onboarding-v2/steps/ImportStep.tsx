import { ArrowRight, Download } from 'lucide-react'
import type { UseFormReturn } from 'react-hook-form'
import { Button } from '@/components/ui/button'
import { FormField, FormItem, FormMessage } from '@/components/ui/form'
import { ChromeProfileTile } from '../components/ChromeProfileTile'
import { ChromeQuitNotice } from '../components/ChromeQuitNotice'
import { DisplayHeading, Em, StepCopy } from '../components/DisplayHeading'
import { ImportedSummaryCard } from '../components/ImportedSummaryCard'
import { ImportingProgressCard } from '../components/ImportingProgressCard'
import { MacKeychainNotice } from '../components/MacKeychainNotice'
import { StepWrap } from '../components/StepWrap'
import {
  CHROME_PROFILES,
  type ChromeProfile,
  sumLoginsFor,
  sumSitesFor,
} from '../onboarding-v2.helpers'
import type { OnboardingFormValues } from '../onboarding-v2.schemas'
import type { ImportPhase } from '../onboarding-v2.types'

interface ImportStepProps {
  phase: ImportPhase
  progress: number
  form: UseFormReturn<OnboardingFormValues>
  onQuitChrome: () => void
  onImport: () => void
  onContinue: () => void
}

/**
 * Step 1. Four sub-phases driven by the `phase` prop:
 *
 *   pre-quit   : amber notice + "Quit Chrome" button
 *   picker     : RHF-managed profile picker + Keychain notice + Import button
 *   importing  : animated progress card (parent owns the timer)
 *   imported   : success bullets + "Connect to Claude" CTA
 *
 * The form lives on the parent so the field stays mounted across all
 * four sub-phases; only the rendered content swaps.
 */
export function ImportStep({
  phase,
  progress,
  form,
  onQuitChrome,
  onImport,
  onContinue,
}: ImportStepProps) {
  const selectedIds = form.watch('selectedProfileIds')
  const selectedProfiles = CHROME_PROFILES.filter((p) =>
    selectedIds.includes(p.id),
  )
  const sites = sumSitesFor(selectedIds)
  const logins = sumLoginsFor(selectedIds)
  const isPickerValid = form.formState.isValid

  return (
    <StepWrap>
      <DisplayHeading>
        Import your <Em>logins</Em>.
      </DisplayHeading>
      <StepCopy>
        BrowserOS copies your saved Chrome sessions so the agent never has to
        log in again. Sessions stay in a local vault on this Mac.
      </StepCopy>

      {phase === 'pre-quit' && <ChromeQuitNotice onQuitChrome={onQuitChrome} />}

      {phase === 'picker' && (
        <>
          <div className="mb-2.5 font-bold text-[12.5px] text-ink-2">
            Choose which Chrome profiles to import
          </div>
          <FormField
            control={form.control}
            name="selectedProfileIds"
            render={({ field }) => (
              <FormItem className="mb-4 flex flex-col gap-2.5">
                {CHROME_PROFILES.map((profile: ChromeProfile) => {
                  const checked = field.value.includes(profile.id)
                  return (
                    <ChromeProfileTile
                      key={profile.id}
                      profile={profile}
                      checked={checked}
                      onCheckedChange={(next) =>
                        field.onChange(
                          next
                            ? [...field.value, profile.id]
                            : field.value.filter((id) => id !== profile.id),
                        )
                      }
                    />
                  )
                })}
                <FormMessage />
              </FormItem>
            )}
          />
          <MacKeychainNotice />
          <Button
            type="button"
            size="lg"
            onClick={onImport}
            disabled={!isPickerValid}
          >
            <Download className="size-4" />
            {selectedProfiles.length === 0
              ? 'Pick at least one profile'
              : `Import ${sites} sites from ${selectedProfiles.length} profile${selectedProfiles.length === 1 ? '' : 's'}`}
          </Button>
        </>
      )}

      {phase === 'importing' && (
        <ImportingProgressCard
          progress={progress}
          total={sites}
          logins={logins}
        />
      )}

      {phase === 'imported' && (
        <>
          <ImportedSummaryCard
            sites={sites}
            profileCount={selectedProfiles.length}
          />
          <Button type="button" size="lg" onClick={onContinue}>
            <ArrowRight className="size-4" />
            Connect to Claude
          </Button>
        </>
      )}
    </StepWrap>
  )
}
