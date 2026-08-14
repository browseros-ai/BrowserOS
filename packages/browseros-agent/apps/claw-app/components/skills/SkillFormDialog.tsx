/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { zodResolver } from '@hookform/resolvers/zod'
import { useQueryClient } from '@tanstack/react-query'
import { type ReactElement, useState } from 'react'
import { useForm } from 'react-hook-form'
import { toast } from 'sonner'
import { z } from 'zod'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
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
import { Textarea } from '@/components/ui/textarea'
import {
  useCreateSkill,
  useSkill,
  useSkills,
  useUpdateSkill,
} from '@/modules/api/skills.hooks'

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
  body: z.string().min(1, 'The SKILL.md body cannot be empty'),
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
  trigger: ReactElement
}

interface EditProps {
  mode: 'edit'
  name: string
  description: string
  site?: string
  body: string
  trigger: ReactElement
}

type SkillFormDialogProps = CreateProps | EditProps

export function SkillFormDialog(props: SkillFormDialogProps) {
  const [open, setOpen] = useState(false)
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger render={props.trigger} />
      <DialogContent className="max-w-lg">
        {props.mode === 'create' ? (
          <CreateForm onClose={() => setOpen(false)} />
        ) : (
          <EditForm {...props} onClose={() => setOpen(false)} />
        )}
      </DialogContent>
    </Dialog>
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
    void toast.promise(
      create
        .mutateAsync({
          name: values.name,
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
        loading: 'Saving the task…',
        success: `Saved /${values.name}`,
        error: 'Could not save the task',
      },
    )
  })

  return (
    <Form {...form}>
      <form onSubmit={onSubmit} className="flex flex-col gap-4">
        <DialogHeader>
          <DialogTitle>New task</DialogTitle>
          <DialogDescription>
            A task is a skill BrowserOS neo links into your agents and you
            re-run by name.
          </DialogDescription>
        </DialogHeader>
        <FormField
          control={form.control}
          name="name"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Name</FormLabel>
              <FormControl>
                <Input
                  placeholder="inbox-sweep"
                  className="font-mono"
                  {...field}
                />
              </FormControl>
              <FormDescription>
                Lowercase letters, digits, and hyphens. Used as the /command.
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
              <FormLabel>Description</FormLabel>
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
              <FormLabel>Site (optional)</FormLabel>
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
              <FormLabel>Steps</FormLabel>
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
              <FormLabel>Learned notes (optional)</FormLabel>
              <FormControl>
                <Textarea rows={3} placeholder="One note per line" {...field} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <DialogFooter>
          <DialogClose
            render={
              <Button variant="outline" type="button">
                Cancel
              </Button>
            }
          />
          <Button type="submit" disabled={create.isPending}>
            Save task
          </Button>
        </DialogFooter>
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
  const form = useForm<z.infer<typeof editSchema>>({
    resolver: zodResolver(editSchema),
    defaultValues: { description, site: site ?? '', body },
  })

  const onSubmit = form.handleSubmit((values) => {
    void toast.promise(
      update
        .mutateAsync({
          name,
          body: {
            description: values.description,
            site: trimmedOrUndefined(values.site),
            body: values.body,
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
        success: `Saved /${name}`,
        error: 'Could not save the task',
      },
    )
  })

  return (
    <Form {...form}>
      <form onSubmit={onSubmit} className="flex flex-col gap-4">
        <DialogHeader>
          <DialogTitle>Edit {name}</DialogTitle>
          <DialogDescription>
            Editing the SKILL.md re-links it into your agents.
          </DialogDescription>
        </DialogHeader>
        <FormField
          control={form.control}
          name="description"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Description</FormLabel>
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
              <FormLabel>Site</FormLabel>
              <FormControl>
                <Input {...field} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name="body"
          render={({ field }) => (
            <FormItem>
              <FormLabel>SKILL.md</FormLabel>
              <FormControl>
                <Textarea rows={12} className="font-mono text-xs" {...field} />
              </FormControl>
              <FormMessage />
            </FormItem>
          )}
        />
        <DialogFooter>
          <DialogClose
            render={
              <Button variant="outline" type="button">
                Cancel
              </Button>
            }
          />
          <Button type="submit" disabled={update.isPending}>
            Save
          </Button>
        </DialogFooter>
      </form>
    </Form>
  )
}
