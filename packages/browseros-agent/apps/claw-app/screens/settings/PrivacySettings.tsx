import { useQueryClient } from '@tanstack/react-query'
import { ChevronDown, CircleCheck, CircleX, ShieldCheck } from 'lucide-react'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleIndicator,
  CollapsibleTrigger,
} from '@/components/motion/collapsible'
import { Switch } from '@/components/motion/switch'
import { cn } from '@/lib/utils'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { applyTelemetry } from '@/modules/analytics/posthog'
import {
  TELEMETRY_QUERY_KEY,
  useSetTelemetryConsent,
  useTelemetryState,
} from '@/modules/analytics/telemetry.hooks'
import { SettingRow, SettingsSection } from './settings.components'

const SHARE_ID = 'share-usage'

const COLLECTED = [
  'Which AI agents you connect',
  'Which neo screens you open',
  'How you move around these screens, in one in five visits, with everything you type hidden',
]

const NEVER_COLLECTED = [
  'The pages you browse',
  'What you ask your AI agents',
  'What your agents send or get back',
  'Anything on a page',
  'Your tasks, history or playback, even in those visits',
  'Your account or personal details',
]

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
          <CollapsibleContent contentClassName="flex flex-col gap-3 px-[18px] pb-[18px]">
            <p className="text-[14.5px] text-ink-2">
              Only anonymous usage, combined with everyone else’s, to make neo
              better.
            </p>
            <div className="grid gap-3 sm:grid-cols-2">
              <CollectList
                title="We collect"
                items={COLLECTED}
                icon={CircleCheck}
                className="border-green/25 bg-green-tint/60 [&_svg]:text-green-ink"
              />
              <CollectList
                title="We never collect"
                items={NEVER_COLLECTED}
                icon={CircleX}
                className="border-red/25 bg-red-tint/60 [&_svg]:text-red-ink"
              />
            </div>
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

// ---------------------------------------------------------------------------

function CollectList({
  title,
  items,
  icon: Icon,
  className,
}: {
  title: string
  items: string[]
  icon: typeof CircleCheck
  className: string
}) {
  return (
    <section
      aria-label={title}
      className={cn('rounded-xl border p-4', className)}
    >
      <h3 className="mb-2.5 font-semibold text-[14.5px] text-ink">{title}</h3>
      <ul className="flex flex-col gap-2">
        {items.map((item) => (
          <li key={item} className="flex gap-2.5 text-[14px] text-ink-2">
            <Icon aria-hidden className="mt-[3px] size-4 shrink-0" />
            <span>{item}</span>
          </li>
        ))}
      </ul>
    </section>
  )
}
