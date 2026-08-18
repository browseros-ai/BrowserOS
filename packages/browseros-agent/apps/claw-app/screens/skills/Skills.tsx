/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Skill } from '@browseros/claw-api'
import { Plus } from 'lucide-react'
import type { ReactNode } from 'react'
import { useLocation, useNavigate } from 'react-router'
import { DeleteSkillDialog } from '@/components/skills/DeleteSkillDialog'
import { RunSkillButton } from '@/components/skills/RunSkillButton'
import { SkillFormDialog } from '@/components/skills/SkillFormDialog'
import { Button } from '@/components/ui/button'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'
import { cn } from '@/lib/utils'
import { useSkillsScreenData } from './skills.data'
import { formatTokens, skillCommand, successRate } from './skills.helpers'

const CELL_PADDING = 'px-2 py-3 first:pl-4 last:pr-4'

/**
 * Tasks list. A task is a skill BrowserOS neo linked into the connected coding
 * agents; each row shows how often it has run, how clean those runs were, and
 * whether it is getting cheaper. Row click opens the SKILL.md and run history.
 */
export function Skills() {
  const { skills, isLoading, isError } = useSkillsScreenData()
  const navigate = useNavigate()
  const location = useLocation()

  const openSkill = (name: string) =>
    navigate(`/skills/${encodeURIComponent(name)}`, {
      state: { from: location.pathname },
    })

  return (
    <div className="relative mx-auto flex w-full max-w-6xl flex-col gap-6 px-8 pt-8 pb-16">
      <header className="flex items-start justify-between gap-4">
        <div className="flex flex-col gap-1">
          <h1 className="font-extrabold text-3xl leading-tight tracking-tight md:text-4xl">
            Tasks
          </h1>
          <p className="text-ink-2 text-sm">
            Skills BrowserOS neo linked into your coding agents. Re-run one by
            name.
          </p>
        </div>
        <SkillFormDialog
          mode="create"
          trigger={
            <Button size="sm" className="rounded-9">
              <Plus />
              New task
            </Button>
          }
        />
      </header>

      {isError ? (
        <SkillsNotice>
          Could not load your tasks. Check that BrowserOS neo is running and try
          again.
        </SkillsNotice>
      ) : isLoading ? (
        <SkillsSkeleton />
      ) : skills.length === 0 ? (
        <SkillsEmpty />
      ) : (
        <LedgerCard>
          <Table className="table-fixed">
            <TableHeader>
              <TableRow className="border-ledger-border border-b bg-ledger-head hover:bg-ledger-head">
                <TableHead
                  className={cn(
                    CELL_PADDING,
                    'h-auto font-medium text-[12px] text-ledger-head-ink',
                  )}
                >
                  Task
                </TableHead>
                <TableHead
                  className={cn(
                    CELL_PADDING,
                    'h-auto w-24 text-right font-medium text-[12px] text-ledger-head-ink',
                  )}
                >
                  Runs
                </TableHead>
                <TableHead
                  className={cn(
                    CELL_PADDING,
                    'h-auto w-40 text-right font-medium text-[12px] text-ledger-head-ink',
                  )}
                >
                  Tokens saved
                </TableHead>
                <TableHead
                  className={cn(
                    CELL_PADDING,
                    'h-auto w-32 text-right font-medium text-[12px] text-ledger-head-ink',
                  )}
                >
                  Success rate
                </TableHead>
                <TableHead
                  className={cn(
                    CELL_PADDING,
                    'h-auto w-44 font-medium text-[12px] text-ledger-head-ink',
                  )}
                >
                  <span className="sr-only">Actions</span>
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {skills.map((skill) => (
                <SkillRow
                  key={skill.name}
                  skill={skill}
                  onOpen={() => openSkill(skill.name)}
                />
              ))}
            </TableBody>
          </Table>
        </LedgerCard>
      )}
    </div>
  )
}

function SkillRow({ skill, onOpen }: { skill: Skill; onOpen: () => void }) {
  const rate = successRate(skill.cleanRunCount, skill.runCount)
  return (
    <TableRow
      data-testid={`skill-row-${skill.name}`}
      onClick={onOpen}
      className="cursor-pointer border-ledger-divider border-b hover:bg-card-tint"
    >
      <TableCell className={cn(CELL_PADDING, 'align-middle')}>
        <div className="flex flex-col gap-0.5">
          <span className="font-medium font-mono text-[13px] text-primary">
            {skillCommand(skill.name)}
          </span>
          <span className="truncate text-[13px] text-ledger-ink-2">
            {skill.description}
          </span>
        </div>
      </TableCell>
      <TableCell
        className={cn(
          CELL_PADDING,
          'text-right align-middle text-[13px] text-ink',
        )}
      >
        {skill.runCount}
      </TableCell>
      <TableCell className={cn(CELL_PADDING, 'text-right align-middle')}>
        {skill.tokenSavings && skill.tokenSavings.measuredRunCount > 0 ? (
          <div className="flex flex-col items-end gap-0.5">
            <span className="text-[13px] text-ink">
              {formatTokens(Math.max(0, skill.tokenSavings.saved))}
            </span>
            <span className="text-[11px] text-ink-3">saved</span>
          </div>
        ) : (
          <span className="text-[13px] text-ink-3">not measured</span>
        )}
      </TableCell>
      <TableCell className={cn(CELL_PADDING, 'text-right align-middle')}>
        <div className="flex flex-col items-end gap-0.5">
          <span className={cn('font-medium text-[13px]', rate.colorClass)}>
            {rate.hasRuns ? `${rate.percent}%` : 'not run'}
          </span>
          <span className="text-[11px] text-ink-3">
            {skill.cleanRunCount}/{skill.runCount}
          </span>
        </div>
      </TableCell>
      <TableCell
        className={cn(CELL_PADDING, 'align-middle')}
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex items-center justify-end gap-1.5">
          <RunSkillButton name={skill.name} />
          <DeleteSkillDialog
            name={skill.name}
            onDeleted={() => {}}
            trigger={
              <Button
                variant="ghost"
                size="sm"
                className="rounded-9 text-red hover:text-red"
              >
                Delete
              </Button>
            }
          />
        </div>
      </TableCell>
    </TableRow>
  )
}

function LedgerCard({ children }: { children: ReactNode }) {
  return (
    <div className="overflow-clip rounded-9 border border-ledger-border bg-card">
      {children}
    </div>
  )
}

function SkillsNotice({ children }: { children: ReactNode }) {
  return (
    <div className="rounded-9 border border-ledger-border bg-card px-6 py-10 text-center text-ink-2 text-sm">
      {children}
    </div>
  )
}

function SkillsEmpty() {
  return (
    <div className="flex flex-col items-center gap-4 rounded-9 border border-ledger-border border-dashed bg-card px-6 py-16 text-center">
      <div className="flex flex-col gap-1">
        <p className="font-semibold text-base text-ink">No tasks yet</p>
        <p className="mx-auto max-w-md text-ink-2 text-sm">
          When BrowserOS neo turns a session into a repeatable skill it shows up
          here, linked into your coding agents. You can also write one yourself.
        </p>
      </div>
      <SkillFormDialog
        mode="create"
        trigger={
          <Button size="sm" className="rounded-9">
            <Plus />
            New task
          </Button>
        }
      />
    </div>
  )
}

function SkillsSkeleton() {
  return (
    <LedgerCard>
      <div className="divide-y divide-ledger-divider">
        {['s1', 's2', 's3', 's4'].map((id) => (
          <div key={id} className="px-4 py-4">
            <div className="h-4 w-full animate-pulse rounded bg-card-tint" />
          </div>
        ))}
      </div>
    </LedgerCard>
  )
}
