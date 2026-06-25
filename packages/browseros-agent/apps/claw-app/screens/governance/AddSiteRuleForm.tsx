import { zodResolver } from '@hookform/resolvers/zod'
import { Plus, X } from 'lucide-react'
import { useState } from 'react'
import { useForm } from 'react-hook-form'
import { Button } from '@/components/ui/button'
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from '@/components/ui/form'
import { Input } from '@/components/ui/input'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import type { AddSiteRuleVariables } from '@/modules/api/site-rules.hooks'
import {
  type AddSiteRuleValues,
  addSiteRuleDefaults,
  addSiteRuleSchema,
  SITE_RULE_ACTIONS,
  siteRuleActionLabel,
} from './site-rules.helpers'

interface AddSiteRuleFormProps {
  isSubmitting: boolean
  /**
   * Receives validated form values plus optional mutation callbacks
   * so the parent can hand `onSuccess` straight to react-query-kit.
   * Keeping the form mounted until the mutation resolves means a 4xx
   * (when a real backend lands) leaves the user's input intact instead
   * of silently dropping the row.
   */
  onSubmit: (
    values: AddSiteRuleVariables,
    options?: { onSuccess?: () => void },
  ) => void
}

/**
 * Inline "Add a rule" form. Collapsed by default; expands into a
 * react-hook-form + zod backed editor with a select for the action
 * category. Submits via the parent's mutation.
 */
export function AddSiteRuleForm({
  isSubmitting,
  onSubmit,
}: AddSiteRuleFormProps) {
  const [open, setOpen] = useState(false)
  const form = useForm<AddSiteRuleValues>({
    resolver: zodResolver(addSiteRuleSchema),
    defaultValues: addSiteRuleDefaults,
    mode: 'onSubmit',
  })

  const close = () => {
    setOpen(false)
    form.reset(addSiteRuleDefaults)
  }

  const handleSubmit = form.handleSubmit((values) => {
    onSubmit(values, { onSuccess: close })
  })

  if (!open) {
    return (
      <Button
        type="button"
        variant="outline"
        onClick={() => setOpen(true)}
        className="h-11 w-full gap-1.5 border-dashed text-ink-2"
      >
        <Plus className="size-3.5" />
        Add a rule
      </Button>
    )
  }

  return (
    <Form {...form}>
      <form
        onSubmit={handleSubmit}
        className="flex flex-col gap-3 rounded-xl border border-border-strong border-dashed bg-bg-sunken p-3"
      >
        <FormField
          control={form.control}
          name="label"
          render={({ field }) => (
            <FormItem>
              <FormLabel className="text-ink-2 text-xs">Label</FormLabel>
              <FormControl>
                <Input
                  placeholder="What to block, e.g. Wire transfers"
                  autoFocus
                  {...field}
                />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <div className="flex gap-2">
          <FormField
            control={form.control}
            name="domain"
            render={({ field }) => (
              <FormItem className="flex-1">
                <FormLabel className="text-ink-2 text-xs">Domain</FormLabel>
                <FormControl>
                  <Input
                    placeholder="mercury.com"
                    className="font-mono"
                    {...field}
                  />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
          <FormField
            control={form.control}
            name="action"
            render={({ field }) => (
              <FormItem className="w-44">
                <FormLabel className="text-ink-2 text-xs">Action</FormLabel>
                <Select value={field.value} onValueChange={field.onChange}>
                  <FormControl>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                  </FormControl>
                  <SelectContent>
                    {SITE_RULE_ACTIONS.map((action) => (
                      <SelectItem key={action} value={action}>
                        {siteRuleActionLabel(action)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <FormMessage />
              </FormItem>
            )}
          />
        </div>
        <div className="flex justify-end gap-2">
          <Button type="button" variant="outline" size="sm" onClick={close}>
            <X className="size-3.5" />
            Cancel
          </Button>
          <Button type="submit" size="sm" disabled={isSubmitting}>
            <Plus className="size-3.5" />
            {isSubmitting ? 'Adding…' : 'Add rule'}
          </Button>
        </div>
      </form>
    </Form>
  )
}
