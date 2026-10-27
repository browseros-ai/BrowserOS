/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { zodResolver } from '@hookform/resolvers/zod'
import { useQueryClient } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { useForm } from 'react-hook-form'
import { z } from 'zod'
import { Button } from '@/components/motion/button/base'
import { StatefulButton } from '@/components/motion/button/stateful'
import { MorphingModal } from '@/components/motion/morphing-modal'
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
import { Textarea } from '@/components/ui/textarea'
import {
  useCreateSkill,
  useSkill,
  useSkills,
  useUpdateSkill,
} from '@/modules/api/skills.hooks'
import { toastPromise } from '@/modules/toast/toast'
import {
  neoName,
  parseSkillBody,
  skillCommand,
} from '@/screens/skills/skills.helpers'

const createSchema = z.object({
  name: z
    .string()
    .regex(/^[a-z0-9-]+$/, 'Lowercase letters, digits, and hyphens only'),
  description: z.string().min(1, 'A one-line description is required'),
  site: z.string().optional(),
  steps: z.string().optional(),
  learnedNotes: z.string().optional(),
})

const editSchema = z.object({
  description: z.string().min(1, 'A one-line description is required'),
  site: z.string().optional(),
  steps: z.string().optional(),
  learnedNotes: z.string().optional(),
})

function toLines(value: string | undefined): string[] {
  return (value ?? '')
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
}

function trimmedOrUndefined(value: string | undefined): string | undefined {
  const trimmed = value?.trim()
  return trimmed ? trimmed : undefined
}

interface CreateProps {
  mode: 'create'
}

interface EditProps {
  mode: 'edit'
  name: string
  description: string
  site?: string
  body: string
}

type SkillFormDialogProps = (CreateProps | EditProps) & {
  open: boolean
  onOpenChange: (open: boolean) => void
}

/** Writing a skill by hand, or changing one an agent saved. */
export function SkillFormDialog(props: SkillFormDialogProps) {
  const close = () => props.onOpenChange(false)
  return (
    <MorphingModal
      viewId={props.open ? props.mode : null}
      onClose={close}
      placement="center"
      className="max-h-[calc(100dvh-32px)] max-w-lg overflow-y-auto rounded-[22px] border-border-2 bg-card"
    >
      {props.mode === 'create' ? (
        <CreateForm onClose={close} />
      ) : (
        <EditForm {...props} onClose={close} />
      )}
    </MorphingModal>
  )
}

function FormHeader({ title, hint }: { title: string; hint: ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <h2 className="font-bold text-[20px] text-cyanotype-ink tracking-[-0.015em]">
        {title}
      </h2>
      <p className="text-[14px] text-ink-2">{hint}</p>
    </div>
  )
}

function FormFooter({
  onClose,
  saving,
}: {
  onClose: () => void
  saving: boolean
}) {
  return (
    <div className="flex justify-end gap-2 pt-1">
      <Button type="button" variant="ghost" onClick={onClose}>
        Cancel
      </Button>
      <StatefulButton
        type="submit"
        state={saving ? 'loading' : 'idle'}
        loadingText="Saving"
        disabled={saving}
        className="font-semibold"
      >
        Save
      </StatefulButton>
    </div>
  )
}

function CreateForm({ onClose }: { onClose: () => void }) {
  const queryClient = useQueryClient()
  const create = useCreateSkill()
  const form = useForm<z.infer<typeof createSchema>>({
    resolver: zodResolver(createSchema),
    defaultValues: {
      name: '',
      description: '',
      site: '',
      steps: '',
      learnedNotes: '',
    },
  })

  const onSubmit = form.handleSubmit((values) => {
    const name = neoName(values.name)
    toastPromise(
      create
        .mutateAsync({
          name,
          description: values.description,
          site: trimmedOrUndefined(values.site),
          steps: toLines(values.steps),
          learnedNotes: toLines(values.learnedNotes),
        })
        .then(() => {
          void queryClient.invalidateQueries({ queryKey: useSkills.getKey() })
          onClose()
        }),
      {
        loading: 'Saving…',
        success: 'Saved the skill',
        error: 'Couldn’t save this skill',
      },
    )
  })

  return (
    <Form {...form}>
      <form onSubmit={onSubmit} className="flex flex-col gap-4">
        <FormHeader
          title="Make a skill"
          hint="Write the steps once. Your AI agents can then run them again from a short phrase."
        />
        <FormField
          control={form.control}
          name="name"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Phrase</FormLabel>
              <FormControl>
                <div className="flex h-9 w-full items-center overflow-hidden rounded-md border border-input bg-transparent shadow-xs transition-[color,box-shadow] focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/50">
                  <span className="flex h-full select-none items-center bg-card-tint px-2.5 font-mono text-ink-2 text-sm">
                    neo-
                  </span>
                  <Input
                    placeholder="inbox-sweep"
                    className="h-full flex-1 rounded-none border-0 font-mono shadow-none focus-visible:ring-0"
                    {...field}
                  />
                </div>
              </FormControl>
              <FormDescription>
                Its phrase will be “
                {skillCommand(
                  field.value ? neoName(field.value) : 'neo-<name>',
                )}
                ”. Lowercase letters, digits and hyphens.
              </FormDescription>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name="description"
          render={({ field }) => (
            <FormItem>
              <FormLabel>What it does</FormLabel>
              <FormControl>
                <Input
                  placeholder="Check the inbox and draft what is owed"
                  {...field}
                />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name="site"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Website (optional)</FormLabel>
              <FormControl>
                <Input placeholder="mail.google.com" {...field} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name="steps"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Steps neo follows</FormLabel>
              <FormControl>
                <Textarea rows={4} placeholder="One step per line" {...field} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name="learnedNotes"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Things neo learned (optional)</FormLabel>
              <FormControl>
                <Textarea rows={3} placeholder="One note per line" {...field} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormFooter onClose={onClose} saving={create.isPending} />
      </form>
    </Form>
  )
}

function EditForm({
  name,
  description,
  site,
  body,
  onClose,
}: EditProps & { onClose: () => void }) {
  const queryClient = useQueryClient()
  const update = useUpdateSkill()
  const parsed = parseSkillBody(body)
  const form = useForm<z.infer<typeof editSchema>>({
    resolver: zodResolver(editSchema),
    defaultValues: {
      description,
      site: site ?? '',
      steps: parsed.steps.join('\n'),
      learnedNotes: parsed.learnedNotes.join('\n'),
    },
  })

  const onSubmit = form.handleSubmit((values) => {
    toastPromise(
      update
        .mutateAsync({
          name,
          body: {
            description: values.description,
            // An empty string clears the site; the field is always sent.
            site: values.site?.trim() ?? '',
            steps: toLines(values.steps),
            learnedNotes: toLines(values.learnedNotes),
          },
        })
        .then(() => {
          void queryClient.invalidateQueries({
            queryKey: useSkill.getKey({ name }),
          })
          void queryClient.invalidateQueries({ queryKey: useSkills.getKey() })
          onClose()
        }),
      {
        loading: 'Saving…',
        success: 'Saved the skill',
        error: 'Couldn’t save this skill',
      },
    )
  })

  return (
    <Form {...form}>
      <form onSubmit={onSubmit} className="flex flex-col gap-4">
        <FormHeader
          title="Edit skill"
          hint={
            <>
              Your AI agents use the changes the next time they run{' '}
              <span className="font-mono text-[13px] text-accent-ink">
                {skillCommand(name)}
              </span>
              .
            </>
          }
        />
        <FormField
          control={form.control}
          name="description"
          render={({ field }) => (
            <FormItem>
              <FormLabel>What it does</FormLabel>
              <FormControl>
                <Input {...field} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name="site"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Website</FormLabel>
              <FormControl>
                <Input placeholder="Leave empty to clear" {...field} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name="steps"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Steps neo follows</FormLabel>
              <FormControl>
                <Textarea rows={5} placeholder="One step per line" {...field} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name="learnedNotes"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Things neo learned</FormLabel>
              <FormControl>
                <Textarea rows={3} placeholder="One note per line" {...field} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormFooter onClose={onClose} saving={update.isPending} />
      </form>
    </Form>
  )
}
