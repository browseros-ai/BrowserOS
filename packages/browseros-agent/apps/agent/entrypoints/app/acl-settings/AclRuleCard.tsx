import type { AclRule } from '@browseros/shared/types/acl'
import { Globe, Trash2 } from 'lucide-react'
import type { FC } from 'react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
import { cn } from '@/lib/utils'

interface AclRuleCardProps {
  rule: AclRule
  onToggle: (id: string, enabled: boolean) => void
  onDelete: (id: string) => void
}

export const AclRuleCard: FC<AclRuleCardProps> = ({
  rule,
  onToggle,
  onDelete,
}) => {
  const summary =
    rule.description ?? rule.textMatch ?? rule.selector ?? 'Block actions'

  return (
    <div
      className={cn(
        'flex items-center gap-4 rounded-xl border p-4 transition-all',
        rule.enabled
          ? 'border-red-300 bg-red-50/50 dark:border-red-800 dark:bg-red-950/20'
          : 'border-border bg-card opacity-60',
      )}
    >
      <Switch
        checked={rule.enabled}
        onCheckedChange={(checked) => onToggle(rule.id, checked)}
      />

      <div className="flex min-w-0 flex-1 flex-col gap-1">
        <span className="truncate font-medium text-sm">{summary}</span>
        <div className="flex flex-wrap items-center gap-2">
          <Badge variant="secondary" className="gap-1 font-mono text-xs">
            <Globe className="size-3" />
            {rule.sitePattern}
          </Badge>
        </div>
      </div>

      <Button
        variant="ghost"
        size="icon"
        className="shrink-0 text-muted-foreground hover:text-destructive"
        onClick={() => onDelete(rule.id)}
      >
        <Trash2 className="size-4" />
      </Button>
    </div>
  )
}
