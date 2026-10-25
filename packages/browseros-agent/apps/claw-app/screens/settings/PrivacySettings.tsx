import { useQueryClient } from '@tanstack/react-query'
import { ChevronDown, ShieldCheck } from 'lucide-react'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleIndicator,
  CollapsibleTrigger,
} from '@/components/motion/collapsible'
import { Switch } from '@/components/motion/switch'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { applyTelemetry } from '@/modules/analytics/posthog'
import {
  TELEMETRY_QUERY_KEY,
  useSetTelemetryConsent,
  useTelemetryState,
} from '@/modules/analytics/telemetry.hooks'
import { SettingRow, SettingsSection } from './settings.components'

const SHARE_ID = 'share-usage'

/**
 * The one consent switch for usage analytics. It is server-owned, so the
 * same answer governs this app and the local server's telemetry.
 */
export function PrivacySettings() {
  const { data } = useTelemetryState()
  const setConsent = useSetTelemetryConsent()
  const queryClient = useQueryClient()
  const distinctId = data?.distinctId

  const handleChange = (consent: boolean) => {
    if (!distinctId) return
    setConsent.mutate(
      { consent },
      {
        onSuccess: (state) => {
          queryClient.setQueryData(TELEMETRY_QUERY_KEY, state)
          // Follows the server's effective state (it honours the kill switch),
          // ordered so the opt-out event still sends while capture is live
          // and the opt-in event sends after it starts.
          if (state.enabled) {
            applyTelemetry({ distinctId: state.distinctId, enabled: true })
            track(AnalyticsEvent.OptOutToggled, { enabled: true })
          } else {
            track(AnalyticsEvent.OptOutToggled, { enabled: false })
            applyTelemetry({ distinctId: state.distinctId, enabled: false })
          }
        },
      },
    )
  }

  return (
    <SettingsSection
      title="Privacy"
      after={
        <Collapsible className="rounded-2xl border border-border-2 bg-card shadow-card">
          <CollapsibleTrigger className="flex w-full items-center gap-2.5 rounded-2xl px-[18px] py-3.5 text-left font-semibold text-[15px] text-ink-2 outline-none hover:text-cyanotype-ink focus-visible:ring-2 focus-visible:ring-ring">
            <ShieldCheck aria-hidden className="size-[15px]" />
            <span className="flex-1">What we collect</span>
            <CollapsibleIndicator>
              <ChevronDown aria-hidden className="size-4" />
            </CollapsibleIndicator>
          </CollapsibleTrigger>
          <CollapsibleContent contentClassName="px-[18px] pb-[18px]">
            <p className="max-w-[68ch] text-[14.5px] text-ink-2">
              BrowserOS neo collects anonymous, aggregate usage (which AI agents
              connect and which screens you open) to improve the product. It
              never collects the pages you browse, your prompts, what your AI
              agents send or get back, or any page content. One in five
              analytics sessions may also record interactions with these
              screens. All inputs are masked, and task, history and playback
              content is blocked from those recordings. No account or personal
              data is used.
            </p>
          </CollapsibleContent>
        </Collapsible>
      }
    >
      <SettingRow
        id={SHARE_ID}
        title="Share anonymous usage"
        description="Helps us improve neo. We never see the pages you visit, what you ask, or what’s on a page."
        control={
          <Switch
            ariaLabel="Share anonymous usage"
            ariaDescribedBy={`${SHARE_ID}-description`}
            checked={data?.consent ?? false}
            onCheckedChange={handleChange}
            disabled={!distinctId || setConsent.isPending}
          />
        }
      >
        {setConsent.isError ? (
          <p role="alert" className="text-[13.5px] text-red-ink">
            Couldn’t save that. Try again.
          </p>
        ) : null}
      </SettingRow>
    </SettingsSection>
  )
}
