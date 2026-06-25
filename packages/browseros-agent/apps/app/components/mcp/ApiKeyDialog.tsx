import { zodResolver } from '@hookform/resolvers/zod'
import { Loader2 } from 'lucide-react'
import { type FC, useEffect } from 'react'
import { useForm } from 'react-hook-form'
import { z } from 'zod/v3'
import { McpServerIcon } from '@/components/mcp/McpServerIcon'
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
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from '@/components/ui/form'
import { Input } from '@/components/ui/input'

const formSchema = z.object({
  apiKey: z.string().min(1, 'API key is required'),
})

type FormValues = z.infer<typeof formSchema>

export interface ApiKeyDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  serverName: string
  onSubmit: (apiKey: string) => void
  isSubmitting?: boolean
}

export const ApiKeyDialog: FC<ApiKeyDialogProps> = ({
  open,
  onOpenChange,
  serverName,
  onSubmit,
  isSubmitting,
}) => {
  const form = useForm<FormValues>({
    resolver: zodResolver(formSchema),
    defaultValues: {
      apiKey: '',
    },
  })

  // biome-ignore lint/correctness/useExhaustiveDependencies: reset form when dialog closes
  useEffect(() => {
    if (!open) form.reset()
  }, [open])

  const handleSubmit = (values: FormValues) => {
    onSubmit(values.apiKey)
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md">
        <DialogHeader>
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg bg-[var(--accent-orange)]/10">
              <McpServerIcon
                serverName={serverName}
                size={20}
                className="text-[var(--accent-orange)]"
              />
            </div>
            <div>
              <DialogTitle>Connect {serverName}</DialogTitle>
              <DialogDescription>
                Enter your {serverName} API key to connect
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>

        <Form {...form}>
          <form
            onSubmit={form.handleSubmit(handleSubmit)}
            className="space-y-4"
          >
            <FormField
              control={form.control}
              name="apiKey"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>API Key</FormLabel>
                  <FormControl>
                    <Input
                      type="password"
                      placeholder="Paste your API key here"
                      autoComplete="off"
                      {...field}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => onOpenChange(false)}
                disabled={isSubmitting}
              >
                Cancel
              </Button>
              <Button
                type="submit"
                className="bg-[var(--accent-orange)] text-white hover:bg-[var(--accent-orange-bright)]"
                disabled={isSubmitting}
              >
                {isSubmitting ? (
                  <>
                    <Loader2 className="h-4 w-4 animate-spin" />
                    Connecting...
                  </>
                ) : (
                  'Connect'
                )}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}
