import type { AclRule } from '@browseros/shared/types/acl'
import { Plus, ShieldAlert } from 'lucide-react'
import { type FC, useEffect, useState } from 'react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { fetchServerAclRules, updateServerAclRules } from '@/lib/acl/api'
import { aclRulesStorage } from '@/lib/acl/storage'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'
import { AclRuleCard } from './AclRuleCard'
import { NewAclRuleDialog } from './NewAclRuleDialog'

export const AclSettingsPage: FC = () => {
  const [rules, setRules] = useState<AclRule[]>([])
  const { baseUrl, isLoading: urlLoading } = useAgentServerUrl()

  useEffect(() => {
    aclRulesStorage.getValue().then(setRules)
    const unwatch = aclRulesStorage.watch(setRules)
    return () => unwatch()
  }, [])

  useEffect(() => {
    if (!baseUrl || urlLoading) return

    const resolvedBaseUrl = baseUrl
    let cancelled = false

    async function bootstrapServerAcl() {
      try {
        const [localRules, serverRules] = await Promise.all([
          aclRulesStorage.getValue(),
          fetchServerAclRules(resolvedBaseUrl),
        ])

        if (cancelled) return

        if (
          serverRules.length === 0 &&
          localRules.some((rule) => rule.enabled)
        ) {
          await updateServerAclRules(resolvedBaseUrl, localRules)
        }
      } catch (error) {
        if (!cancelled) {
          void error
        }
      }
    }

    void bootstrapServerAcl()

    return () => {
      cancelled = true
    }
  }, [baseUrl, urlLoading])

  const saveRules = async (next: AclRule[]) => {
    setRules(next)
    await aclRulesStorage.setValue(next)

    if (!baseUrl) return

    try {
      await updateServerAclRules(baseUrl, next)
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : 'Failed to sync ACL rules to the server',
      )
    }
  }

  const handleAddRule = (rule: AclRule) => {
    void saveRules([...rules, rule])
  }

  const handleToggle = (id: string, enabled: boolean) => {
    void saveRules(rules.map((r) => (r.id === id ? { ...r, enabled } : r)))
  }

  const handleDelete = (id: string) => {
    void saveRules(rules.filter((r) => r.id !== id))
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
