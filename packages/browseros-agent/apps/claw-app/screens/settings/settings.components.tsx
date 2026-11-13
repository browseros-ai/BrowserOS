import type { ReactNode } from 'react'
import { Card } from '@/components/motion/card'

/** One titled group of settings, the way every section on the page looks. */
export function SettingsSection({
  title,
  children,
  after,
}: {
  title: string
  children: ReactNode
  /** Shown under the card, such as an explanation that opens. */
  after?: ReactNode
}) {
  return (
    <section aria-label={title} className="flex flex-col gap-3.5">
      <h2 className="font-bold text-[19px] text-cyanotype-ink tracking-[-0.015em]">
        {title}
      </h2>
      {/* No overflow clip: the beUI select draws its options inside the
          field, so a clipped card would cut them off. */}
      <Card
        size="compact"
        className="gap-0 border-border-2 bg-card py-0 shadow-card"
      >
        <div className="flex flex-col divide-y divide-border-2">{children}</div>
      </Card>
      {after}
    </section>
  )
}

/** A setting's title and plain explanation, with its control on the right. */
export function SettingRow({
  id,
  title,
  description,
  control,
  children,
}: {
  /** Prefix for the title and description ids the control points at. */
  id: string
  title: ReactNode
  description: ReactNode
  control?: ReactNode
  /** Anything below the row, such as an error or an extra field. */
  children?: ReactNode
}) {
  return (
    <div className="flex flex-col gap-3 px-5 py-4">
      <div className="flex flex-wrap items-center justify-between gap-x-6 gap-y-3">
        <div className="flex min-w-0 flex-1 basis-[280px] flex-col gap-1">
          <span
            id={`${id}-title`}
            className="font-semibold text-[15.5px] text-cyanotype-ink"
          >
            {title}
          </span>
          <div
            id={`${id}-description`}
            className="max-w-[62ch] text-[14px] text-ink-2"
          >
            {description}
          </div>
        </div>
        {control ? <div className="shrink-0">{control}</div> : null}
      </div>
      {children}
    </div>
  )
}
