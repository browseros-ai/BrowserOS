import {
  Bot,
  Camera,
  Code,
  Database,
  Eye,
  Hand,
  MousePointerClick,
  Navigation,
} from 'lucide-react'
import { type FC, useEffect, useState } from 'react'
import { Switch } from '@/components/ui/switch'
import {
  normalizeToolApprovalConfig,
  toolApprovalConfigStorage,
} from '@/lib/tool-approvals/storage'
import {
  TOOL_CATEGORIES,
  type ToolApprovalConfig,
} from '@/lib/tool-approvals/types'

const CATEGORY_ICONS: Record<string, typeof Hand> = {
  input: MousePointerClick,
  navigation: Navigation,
  observation: Eye,
  screenshots: Camera,
  scripts: Code,
  'data-modification': Database,
  assistant: Bot,
}

export const ToolApprovalsPage: FC = () => {
  const [config, setConfig] = useState<ToolApprovalConfig>({ categories: {} })

  useEffect(() => {
    const applyConfig = (value: ToolApprovalConfig) =>
      setConfig(normalizeToolApprovalConfig(value))

    toolApprovalConfigStorage.getValue().then(applyConfig)
    const unwatch = toolApprovalConfigStorage.watch(applyConfig)
    return () => unwatch()
  }, [])

  const allEnabled =
    TOOL_CATEGORIES.length > 0 &&
    TOOL_CATEGORIES.every((category) => config.categories[category.id] === true)

  const toggleCategory = (categoryId: string, enabled: boolean) => {
    const next = {
      ...config,
      categories: { ...config.categories, [categoryId]: enabled },
    }
    setConfig(next)
    toolApprovalConfigStorage.setValue(normalizeToolApprovalConfig(next))
  }

  const toggleAll = (enabled: boolean) => {
    const categories: Record<string, boolean> = {}
    for (const cat of TOOL_CATEGORIES) {
      categories[cat.id] = enabled
    }
    const next = { ...config, categories }
    setConfig(next)
    toolApprovalConfigStorage.setValue(normalizeToolApprovalConfig(next))
  }

  return (
    <div className="space-y-6">
      <div>
        <h2 className="font-semibold text-xl tracking-tight">Tool Approvals</h2>
        <p className="text-muted-foreground text-sm">
          Require human approval before the agent executes certain actions.
          Changes apply immediately.
        </p>
      </div>

      <div className="flex items-center justify-between rounded-lg border bg-card p-4">
        <div className="space-y-0.5">
          <div className="font-medium text-sm">Require approval for all</div>
          <div className="text-muted-foreground text-xs">
            Toggle all categories at once
          </div>
        </div>
        <Switch checked={allEnabled} onCheckedChange={toggleAll} />
      </div>

      <div className="space-y-3">
        {TOOL_CATEGORIES.map((category) => {
          const Icon = CATEGORY_ICONS[category.id] ?? Hand
          const enabled = config.categories[category.id] ?? false

          return (
            <div
              key={category.id}
              className="flex items-start gap-4 rounded-lg border bg-card p-4 transition-colors"
            >
              <div className="mt-0.5 flex size-9 shrink-0 items-center justify-center rounded-md bg-muted">
                <Icon className="size-4 text-muted-foreground" />
              </div>
              <div className="min-w-0 flex-1 space-y-1">
                <div className="flex items-center gap-2">
                  <span className="font-medium text-sm">{category.name}</span>
                </div>
                <p className="text-muted-foreground text-xs">
                  {category.description}
                </p>
              </div>
              <Switch
                checked={enabled}
                onCheckedChange={(checked) =>
                  toggleCategory(category.id, checked)
                }
              />
            </div>
          )
        })}
      </div>
    </div>
  )
}
