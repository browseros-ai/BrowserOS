import type { AclRule } from '@browseros/shared/types/acl'
import { Plus, ShieldAlert } from 'lucide-react'
import { type FC, useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import { aclRulesStorage } from '@/lib/acl/storage'
import { AclRuleCard } from './AclRuleCard'
import { NewAclRuleDialog } from './NewAclRuleDialog'

export const AclSettingsPage: FC = () => {
  const [rules, setRules] = useState<AclRule[]>([])

  useEffect(() => {
    aclRulesStorage.getValue().then(setRules)
    const unwatch = aclRulesStorage.watch(setRules)
    return () => unwatch()
  }, [])

  const saveRules = (next: AclRule[]) => {
    setRules(next)
    aclRulesStorage.setValue(next)
  }

  const handleAddRule = (rule: AclRule) => {
    saveRules([...rules, rule])
  }

  const handleToggle = (id: string, enabled: boolean) => {
    saveRules(rules.map((r) => (r.id === id ? { ...r, enabled } : r)))
  }

  const handleDelete = (id: string) => {
    saveRules(rules.filter((r) => r.id !== id))
  }

  return (
    <div className="mx-auto max-w-2xl p-6">
      <div className="mb-6 flex items-center justify-between">
        <div>
          <h1 className="font-semibold text-xl">ACL Rules</h1>
          <p className="mt-1 text-muted-foreground text-sm">
            Describe what the agent should avoid on a site and BrowserOS will
            block matching actions.
          </p>
        </div>
        <NewAclRuleDialog onSave={handleAddRule}>
          <Button size="sm">
            <Plus className="mr-1 size-4" />
            Add Rule
          </Button>
        </NewAclRuleDialog>
      </div>

      {rules.length === 0 ? (
        <div className="flex flex-col items-center gap-3 rounded-xl border border-dashed p-12 text-center">
          <ShieldAlert className="size-10 text-muted-foreground" />
          <div>
            <p className="font-medium">No ACL rules defined</p>
            <p className="mt-1 text-muted-foreground text-sm">
              Add a plain-English rule like &ldquo;payments and checkout&rdquo;
              or &ldquo;send email&rdquo; and BrowserOS will apply broad safety
              blocking on that site.
            </p>
          </div>
          <NewAclRuleDialog onSave={handleAddRule}>
            <Button variant="outline" size="sm">
              <Plus className="mr-1 size-4" />
              Add your first rule
            </Button>
          </NewAclRuleDialog>
        </div>
      ) : (
        <div className="flex flex-col gap-3">
          {rules.map((rule) => (
            <AclRuleCard
              key={rule.id}
              rule={rule}
              onToggle={handleToggle}
              onDelete={handleDelete}
            />
          ))}
        </div>
      )}
    </div>
  )
}
