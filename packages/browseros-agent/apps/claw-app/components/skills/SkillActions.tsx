/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Skill } from '@browseros/claw-api'
import { Pencil, Trash2 } from 'lucide-react'
import { useState } from 'react'
import { OverflowActions } from '@/components/motion/overflow-actions'
import { useSkill } from '@/modules/api/skills.hooks'
import { DeleteSkillDialog } from './DeleteSkillDialog'
import { SkillFormDialog } from './SkillFormDialog'

interface SkillActionsProps {
  skill: Pick<Skill, 'name' | 'description' | 'site'>
  /** The saved task's file, when the page already has it. */
  body?: string
  onDeleted?: () => void
}

/** Edit and Delete behind one button, so the row leads with Copy phrase. */
export function SkillActions({ skill, body, onDeleted }: SkillActionsProps) {
  const [editing, setEditing] = useState(false)
  const [deleting, setDeleting] = useState(false)

  return (
    <>
      <OverflowActions
        size="sm"
        openLabel={`More for ${skill.description}`}
        closeLabel="Close"
        primaryActions={[]}
        overflowActions={[
          {
            id: 'edit',
            label: 'Edit',
            icon: <Pencil className="size-4" />,
            onClick: () => setEditing(true),
          },
          {
            id: 'delete',
            label: 'Delete',
            icon: <Trash2 className="size-4" />,
            onClick: () => setDeleting(true),
          },
        ]}
        collapseOnAction
        classNames={{
          root: 'shrink-0',
          track: 'border-border-strong',
          toggle: 'bg-transparent text-ink-2 hover:bg-bg-sunken',
          action: 'font-semibold text-ink-2',
        }}
      />
      {editing ? (
        body === undefined ? (
          <EditLoader skill={skill} onClose={() => setEditing(false)} />
        ) : (
          <SkillFormDialog
            mode="edit"
            open
            onOpenChange={setEditing}
            name={skill.name}
            description={skill.description}
            site={skill.site}
            body={body}
          />
        )
      ) : null}
      <DeleteSkillDialog
        name={skill.name}
        description={skill.description}
        open={deleting}
        onOpenChange={setDeleting}
        onDeleted={onDeleted}
      />
    </>
  )
}

// ---------------------------------------------------------------------------

/** Mounted only while editing from the list, so the file loads on demand. */
function EditLoader({
  skill,
  onClose,
}: {
  skill: SkillActionsProps['skill']
  onClose: () => void
}) {
  const query = useSkill({ variables: { name: skill.name } })
  return (
    <SkillFormDialog
      mode="edit"
      open={query.data !== undefined}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
      name={skill.name}
      description={skill.description}
      site={skill.site}
      body={query.data?.body ?? ''}
    />
  )
}
