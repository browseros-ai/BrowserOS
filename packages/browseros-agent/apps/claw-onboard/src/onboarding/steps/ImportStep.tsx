import { AlertTriangle, ArrowRight, Download, RefreshCw } from 'lucide-react'
import type { UseFormReturn } from 'react-hook-form'
import { Button } from '@/components/ui/button'
import { FormField, FormItem, FormMessage } from '@/components/ui/form'
import type {
  BrowserOSImportItem,
  BrowserOSOnboardingState,
} from '../browseros-onboarding-api'
import { ChromeQuitNotice } from '../components/ChromeQuitNotice'
import { DisplayHeading, Em, StepCopy } from '../components/DisplayHeading'
import { ImportedSummaryCard } from '../components/ImportedSummaryCard'
import { ImportItemChecklist } from '../components/ImportItemChecklist'
import { ImportingProgressCard } from '../components/ImportingProgressCard'
import { ImportSourceTile } from '../components/ImportSourceTile'
import { MacKeychainNotice } from '../components/MacKeychainNotice'
import { StepWrap } from '../components/StepWrap'
import {
  completedImportItemCount,
  importItemLabel,
  importItemListLabel,
  importProgressTotal,
  sanitizeImportSelection,
  selectableItemsForSource,
  selectedSourceById,
} from '../onboarding-v2.helpers'
import type { OnboardingFormValues } from '../onboarding-v2.schemas'
import type { ImportPhase } from '../onboarding-v2.types'

interface ImportStepProps {
  phase: ImportPhase
  state: BrowserOSOnboardingState
  form: UseFormReturn<OnboardingFormValues>
  onQuitChrome: () => void
  onImport: () => void
  onRefresh: () => void
  onContinue: () => void
}

function importButtonLabelFor(
  hasSource: boolean,
  hasSupportedItems: boolean,
  checkedItemCount: number,
  sourceName: string,
): string {
  if (!hasSource) return 'Pick an import source'
  if (!hasSupportedItems) return 'No supported import items'
  if (checkedItemCount === 0) return 'Select items to import'
  return `Import ${checkedItemCount} ${
    checkedItemCount === 1 ? 'item' : 'items'
  } from ${sourceName}`
}

/** Renders the browser import step across quit, picker, progress, and success states. */
export function ImportStep({
  phase,
  state,
  form,
  onQuitChrome,
  onImport,
  onRefresh,
  onContinue,
}: ImportStepProps) {
  const selectedSourceId = form.watch('selectedSourceId')
  const selectedSource = selectedSourceById(state.sources, selectedSourceId)
  const checkedItems = selectedSource
    ? sanitizeImportSelection(selectedSource, form.watch('selectedItems'))
    : []
  const sourceName =
    selectedSource?.profileName || selectedSource?.browserName || 'source'
  const isDetecting = state.status === 'detecting'
  const hasSupportedItems = (selectedSource?.supportedItems.length ?? 0) > 0
  const isPickerValid =
    Boolean(selectedSource) &&
    hasSupportedItems &&
    checkedItems.length > 0 &&
    !isDetecting
  const completedItems = completedImportItemCount(state.progress)
  const totalItems = selectedSource
    ? importProgressTotal(checkedItems.length, state.progress)
    : (state.progress?.totalItems ?? 0)
  const currentItemLabel = state.progress?.currentItem
    ? importItemLabel(state.progress.currentItem)
    : undefined
  const importedItems = state.progress?.completedItems ?? []
  const importedItemSummary = state.progress
    ? importedItems.length
      ? importItemListLabel(importedItems)
      : 'No completed items reported'
    : 'No item details reported'
  const importButtonLabel = importButtonLabelFor(
    Boolean(selectedSource),
    hasSupportedItems,
    checkedItems.length,
    sourceName,
  )

  function toggleImportItem(item: BrowserOSImportItem) {
    if (!selectedSource) return
    const currentItems = sanitizeImportSelection(
      selectedSource,
      form.getValues('selectedItems'),
    )
    const nextItems = currentItems.includes(item)
      ? currentItems.filter((currentItem) => currentItem !== item)
      : sanitizeImportSelection(selectedSource, [...currentItems, item])
    form.setValue('selectedItems', nextItems, {
      shouldDirty: true,
      shouldValidate: true,
    })
  }

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
          <div className="mb-2.5 flex items-center justify-between gap-3">
            <div className="font-bold text-[12.5px] text-ink-2">
              {isDetecting
                ? 'Detecting import sources'
                : 'Choose a browser profile to import'}
            </div>
            <Button
              type="button"
              size="sm"
              variant="ghost"
              onClick={onRefresh}
              disabled={isDetecting}
            >
              <RefreshCw
                className={`size-3.5 ${isDetecting ? 'animate-spin' : ''}`}
              />
              Refresh
            </Button>
          </div>
          <FormField
            control={form.control}
            name="selectedSourceId"
            render={({ field }) => (
              <FormItem
                className="mb-4 flex flex-col gap-2.5"
                role="radiogroup"
              >
                {state.sources.map((source) => (
                  <ImportSourceTile
                    key={source.id}
                    source={source}
                    selected={field.value === source.id}
                    onSelect={() => {
                      if (field.value === source.id) return
                      field.onChange(source.id)
                      form.setValue(
                        'selectedItems',
                        selectableItemsForSource(source),
                        {
                          shouldDirty: true,
                          shouldValidate: true,
                        },
                      )
                    }}
                  />
                ))}
                {!isDetecting && state.sources.length === 0 && (
                  <div className="rounded-xl border border-border-2 bg-card p-4 text-[12.5px] text-ink-2">
                    No import sources detected.
                  </div>
                )}
                <FormMessage />
              </FormItem>
            )}
          />
          {selectedSource && hasSupportedItems && (
            <ImportItemChecklist
              items={selectedSource.supportedItems}
              checkedItems={checkedItems}
              onToggle={toggleImportItem}
            />
          )}
          {state.error && (
            <div className="mb-4 rounded-xl border border-amber/30 bg-amber-tint p-4 text-[12.5px] text-ink-2">
              {state.error.message}
            </div>
          )}
          <MacKeychainNotice />
          <Button
            type="button"
            size="lg"
            onClick={onImport}
            disabled={!isPickerValid}
          >
            <Download className="size-4" />
            {importButtonLabel}
          </Button>
        </>
      )}

      {phase === 'importing' && (
        <ImportingProgressCard
          currentItemLabel={currentItemLabel}
          progress={completedItems}
          total={totalItems}
        />
      )}

      {phase === 'failed' && (
        <>
          <div className="mb-4 rounded-xl border border-amber/30 bg-amber-tint p-4">
            <div className="mb-2 flex items-center gap-2 font-bold text-[13px] text-ink">
              <AlertTriangle className="size-4 text-amber" />
              Import failed
            </div>
            <div className="text-[12.5px] text-ink-2">
              {state.error?.message ??
                'BrowserOS could not finish importing this profile.'}
            </div>
          </div>
          <div className="flex flex-wrap gap-2.5">
            <Button
              type="button"
              size="lg"
              onClick={onImport}
              disabled={!isPickerValid}
            >
              <Download className="size-4" />
              Try import again
            </Button>
            <Button type="button" size="lg" variant="ghost" onClick={onRefresh}>
              <RefreshCw className="size-4" />
              Refresh sources
            </Button>
          </div>
        </>
      )}

      {phase === 'imported' && (
        <>
          <ImportedSummaryCard
            importedItemCount={completedItems}
            itemSummary={importedItemSummary}
            sourceName={sourceName}
          />
          <Button type="button" size="lg" onClick={onContinue}>
            <ArrowRight className="size-4" />
            Continue
          </Button>
        </>
      )}
    </StepWrap>
  )
}
