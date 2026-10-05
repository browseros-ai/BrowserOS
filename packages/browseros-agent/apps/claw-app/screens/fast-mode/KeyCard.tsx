/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Adding, replacing and forgetting the key. The key is never rendered back:
 * once stored, only its last characters are shown so the user can tell which
 * one is in place.
 */

import { useQueryClient } from '@tanstack/react-query'
import { useState } from 'react'
import { toast } from 'sonner'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Spinner } from '@/components/ui/spinner'
import {
  jevModeKey,
  useForgetJevCredential,
  useSaveJevCredential,
} from '@/modules/api/jev-mode.hooks'
import { credentialSchema } from './fast-mode.schemas'

interface KeyCardProps {
  configured: boolean
  fingerprint: string
}

export function KeyCard({ configured, fingerprint }: KeyCardProps) {
  const [credential, setCredential] = useState('')
  const [problem, setProblem] = useState<string | null>(null)
  const save = useSaveJevCredential()
  const forget = useForgetJevCredential()
  const queryClient = useQueryClient()

  const onSave = async () => {
    const parsed = credentialSchema.safeParse({ credential })
    if (!parsed.success) {
      setProblem(parsed.error.issues[0]?.message ?? 'Paste a key.')
      return
    }
    setProblem(null)
    try {
      const next = await save.mutateAsync(parsed.data)
      queryClient.setQueryData(jevModeKey, next)
      // Cleared immediately: the page holds the key only until it is stored.
      setCredential('')
      toast.success('Key checked and saved.')
    } catch (error) {
      // The provider's own wording, which is what says whether the key is
      // wrong, the account is suspended, or the network is blocked.
      const message =
        error instanceof Error
          ? error.message
          : 'The provider rejected the key.'
      setProblem(message)
      toast.error('That key was not accepted.')
    }
  }

  const onForget = async () => {
    try {
      const next = await forget.mutateAsync()
      queryClient.setQueryData(jevModeKey, next)
      toast.success('Key forgotten.')
    } catch {
      toast.error('Could not forget the key.')
    }
  }

  return (
    <Card className="flex flex-col gap-4 p-5">
      <div className="flex flex-col gap-1">
        <h2 className="font-medium text-sm">Decision model key</h2>
        <p className="text-muted-foreground text-sm">
          {configured
            ? `Stored, ending ${fingerprint}. It is checked against the provider before it is saved.`
            : 'Checked against the provider before it is saved, so you find out here rather than mid task.'}
        </p>
      </div>

      <div className="flex flex-col gap-2">
        <Label htmlFor="jev-credential">
          {configured ? 'Replace key' : 'Key'}
        </Label>
        <div className="flex gap-2">
          <Input
            id="jev-credential"
            type="password"
            autoComplete="off"
            spellCheck={false}
            placeholder={configured ? 'Paste a new key' : 'Paste your key'}
            value={credential}
            onChange={(event) => {
              setCredential(event.target.value)
              setProblem(null)
            }}
          />
          <Button onClick={onSave} disabled={save.isPending}>
            {save.isPending ? <Spinner /> : null}
            {save.isPending ? 'Checking' : configured ? 'Replace' : 'Add key'}
          </Button>
        </div>
        {problem ? (
          <p className="text-destructive text-sm" role="alert">
            {problem}
          </p>
        ) : null}
      </div>

      {configured ? (
        <AlertDialog>
          <AlertDialogTrigger
            render={
              <Button
                variant="outline"
                className="self-start"
                disabled={forget.isPending}
              >
                Forget key
              </Button>
            }
          />
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>Forget this key?</AlertDialogTitle>
              <AlertDialogDescription>
                Goal-driven browsing switches off and you will need to paste the
                key again to use it. To switch off without losing the key, use
                the toggle instead.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Keep it</AlertDialogCancel>
              <AlertDialogAction onClick={onForget}>
                Forget key
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      ) : null}
    </Card>
  )
}
