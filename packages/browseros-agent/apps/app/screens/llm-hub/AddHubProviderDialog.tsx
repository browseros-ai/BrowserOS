import { zodResolver } from '@hookform/resolvers/zod'
import type { FC } from 'react'
import { useEffect } from 'react'
import { useForm } from 'react-hook-form'
import { z } from 'zod/v3'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from '@/components/ui/form'
import { Input } from '@/components/ui/input'
import { HUB_PROVIDER_ADDED_EVENT } from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'
import type { LlmHubProvider } from './models'

const formSchema = z.object({
  name: z
    .string()
    .min(1, 'Provider name is required')
    .max(50, 'Name must be 50 characters or less'),
  url: z
    .string()
    .min(1, 'URL is required')
    .refine(
      (val) => {
        try {
          new URL(val.startsWith('http') ? val : `https://${val}`)
          return true
        } catch {
          return false
        }
      },
      { message: 'Please enter a valid URL' },
    ),
})

type FormValues = z.infer<typeof formSchema>

export interface AddHubProviderDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  initialValues?: LlmHubProvider | null
  onSave: (provider: LlmHubProvider) => void
}

export const AddHubProviderDialog: FC<AddHubProviderDialogProps> = ({
  open,
  onOpenChange,
  initialValues,
  onSave,
}) => {
  const isEditing = !!initialValues

  const form = useForm<FormValues>({
    resolver: zodResolver(formSchema),
    defaultValues: { name: '', url: '' },
  })

  useEffect(() => {
    if (open) {
      form.reset(
        initialValues
          ? { name: initialValues.name, url: initialValues.url }
          : { name: '', url: '' },
      )
    }
  }, [open, initialValues, form])

  const onSubmit = (values: FormValues) => {
    let url = values.url.trim()
    if (!url.startsWith('http://') && !url.startsWith('https://')) {
      url = `https://${url}`
    }
    if (url.endsWith('/')) {
      url = url.slice(0, -1)
    }

    onSave({ name: values.name.trim(), url })
    if (!isEditing) {
      track(HUB_PROVIDER_ADDED_EVENT, { name: values.name.trim(), url })
    }
    form.reset()
    onOpenChange(false)
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>
            {isEditing ? 'Edit Provider' : 'Add Custom Provider'}
          </DialogTitle>
          <DialogDescription>
            {isEditing
              ? 'Update your custom AI chat provider.'
              : 'Add a new AI chat provider for quick access.'}
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-4">
            <FormField
              control={form.control}
              name="name"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Provider Name</FormLabel>
                  <FormControl>
                    <Input placeholder="e.g., DeepSeek" {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="url"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>URL</FormLabel>
                  <FormControl>
                    <Input placeholder="e.g., chat.deepseek.com" {...field} />
                  </FormControl>
                  <FormDescription>
                    The web address of the AI chat service
                  </FormDescription>
                  <FormMessage />
                </FormItem>
              )}
            />

            <DialogFooter className="gap-2 pt-2">
              <Button
                type="button"
                variant="outline"
                onClick={() => onOpenChange(false)}
              >
                Cancel
              </Button>
              <Button type="submit">{isEditing ? 'Update' : 'Add'}</Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}
