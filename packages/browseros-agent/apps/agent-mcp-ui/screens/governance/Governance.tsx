import { Check, History, Lock, ShieldCheck } from 'lucide-react'
import type { ComponentType, SVGProps } from 'react'
import { Outlet, useLocation, useNavigate } from 'react-router'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { cn } from '@/lib/utils'
import { useRuns } from '@/modules/api/runs.hooks'
import { countLiveRuns } from './governance.helpers'

interface TabEntry {
  key: 'audit' | 'permissions' | 'site-rules' | 'grants'
  label: string
  Icon: ComponentType<SVGProps<SVGSVGElement>>
}

const TABS: readonly TabEntry[] = [
  { key: 'audit', label: 'Audit', Icon: History },
  { key: 'permissions', label: 'Permissions', Icon: ShieldCheck },
  { key: 'site-rules', label: 'Site Rules', Icon: Lock },
  { key: 'grants', label: 'Grants', Icon: Check },
]

export function Governance() {
  const { data: runs = [] } = useRuns()
  const navigate = useNavigate()
  const location = useLocation()

  const activeKey = (TABS.find((t) => location.pathname.endsWith(`/${t.key}`))
    ?.key ?? 'audit') as TabEntry['key']
  const liveCount = countLiveRuns(runs)

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col px-8 pt-6 pb-20">
      <header className="mb-5 flex items-center gap-3">
        <span className="flex size-11 shrink-0 items-center justify-center rounded-xl bg-accent-tint text-accent">
          <ShieldCheck className="size-5" />
        </span>
        <div className="flex-1">
          <div className="flex items-center gap-2">
            <h1 className="font-extrabold text-2xl text-ink tracking-tight">
              Governance
            </h1>
            {liveCount > 0 && (
              <span className="inline-flex items-center gap-1.5 rounded-full bg-green-tint px-2.5 py-0.5 font-bold text-green text-xs">
                <span
                  aria-hidden
                  className="size-1.5 animate-pulse-dot rounded-full bg-green"
                />
                {liveCount} live
              </span>
            )}
          </div>
          <p className="mt-0.5 text-ink-2 text-sm">
            What your agents are allowed to do, and everything they've done.
          </p>
        </div>
      </header>

      <Tabs
        value={activeKey}
        onValueChange={(value) => navigate(`/governance/${value}`)}
        className="gap-5"
      >
        <TabsList variant="line" className="border-border border-b">
          {TABS.map(({ key, label, Icon }) => (
            <TabsTrigger
              key={key}
              value={key}
              className={cn(
                'gap-2 px-4 py-2 font-semibold text-ink-3 text-sm hover:text-ink',
                'data-active:text-ink',
                'after:!bg-accent after:!bottom-[-1px]',
                '[&_svg]:size-3.5 [&_svg]:text-ink-4 data-active:[&_svg]:text-accent',
              )}
            >
              <Icon />
              {label}
            </TabsTrigger>
          ))}
        </TabsList>
      </Tabs>

      <div className="mt-1">
        <Outlet />
      </div>
    </div>
  )
}
