/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Skill, SkillRun, SkillTokenSavings } from '@browseros/claw-api'
import { ArrowLeft, Check } from 'lucide-react'
import type { ReactNode } from 'react'
import { useNavigate } from 'react-router'
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
import { useSkillDetailData } from './skill-detail.data'
import {
  formatRelativeTime,
  formatTokens,
  skillCommand,
  successRate,
} from './skills.helpers'

const AGENT_LABELS: Record<string, string> = {
  claude_code: 'Claude Code',
  codex: 'Codex',
  cursor: 'Cursor',
  gemini: 'Gemini',
}

function agentLabel(id: string): string {
  return AGENT_LABELS[id] ?? id
}

/** Task detail: the SKILL.md, its token savings and success rate, which agents
 *  it is linked into, and its run history. */
export function SkillDetail() {
  const { detail, isLoading, isError } = useSkillDetailData()
  const navigate = useNavigate()

  return (
    <div className="relative mx-auto flex w-full max-w-5xl flex-col gap-6 px-8 pt-8 pb-16">
      <button
        type="button"
        onClick={() => navigate('/skills')}
        className="inline-flex w-fit items-center gap-1.5 font-medium text-[13px] text-ink-2 transition-colors hover:text-ink"
      >
        <ArrowLeft className="size-3.5" />
        Tasks
      </button>

      {isError ? (
        <Notice>
          Could not load this task. Check that BrowserOS neo is running and try
          again.
        </Notice>
      ) : isLoading || !detail ? (
        <DetailSkeleton />
      ) : (
        <DetailBody
          skill={detail.skill}
          body={detail.body}
          runs={detail.runs}
          tokenSavings={detail.tokenSavings}
          onDeleted={() => navigate('/skills')}
        />
      )}
    </div>
  )
}

function DetailBody({
  skill,
  body,
  runs,
  tokenSavings,
  onDeleted,
}: {
  skill: Skill
  body: string
  runs: SkillRun[]
  tokenSavings: SkillTokenSavings
  onDeleted: () => void
}) {
  return (
    <>
      <header className="flex flex-wrap items-start justify-between gap-4">
        <div className="flex flex-col gap-1">
          <h1 className="font-extrabold font-mono text-2xl text-primary leading-tight tracking-tight md:text-3xl">
            {skillCommand(skill.name)}
          </h1>
          <p className="max-w-2xl text-ink-2 text-sm">{skill.description}</p>
          {skill.site && (
            <p className="text-ink-3 text-xs">operates on {skill.site}</p>
          )}
        </div>
        <div className="flex items-center gap-1.5">
          <RunSkillButton name={skill.name} />
          <SkillFormDialog
            mode="edit"
            name={skill.name}
            description={skill.description}
            site={skill.site}
            body={body}
            trigger={
              <Button variant="outline" size="sm" className="rounded-9">
                Edit
              </Button>
            }
          />
          <DeleteSkillDialog name={skill.name} onDeleted={onDeleted} />
        </div>
      </header>

      <div className="grid grid-cols-1 gap-4 md:grid-cols-3">
        <TokensSavedCard tokenSavings={tokenSavings} />
        <SuccessRateCard skill={skill} />
        <LinkedIntoCard skill={skill} />
      </div>

      <section className="flex flex-col gap-2">
        <h2 className="font-semibold text-ink text-sm">SKILL.md</h2>
        <pre className="overflow-x-auto rounded-9 border border-ledger-border bg-card p-4 font-mono text-[12px] text-ledger-ink leading-relaxed">
          {body}
        </pre>
      </section>

      <RunHistory runs={runs} />
    </>
  )
}

function StatCard({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-3 rounded-9 border border-ledger-border bg-card p-4">
      <h3 className="font-medium text-[12px] text-ledger-ink-2 uppercase tracking-wide">
        {title}
      </h3>
      {children}
    </div>
  )
}

/** Tokens this skill saved versus a screenshot-first agent, and what those
 *  same runs would have cost in other browsers. */
function TokensSavedCard({
  tokenSavings,
}: {
  tokenSavings: SkillTokenSavings
}) {
  if (tokenSavings.measuredRunCount === 0) {
    return (
      <StatCard title="Tokens saved">
        <p className="text-ink-3 text-sm">No measured runs yet.</p>
      </StatCard>
    )
  }
  const saved = Math.max(0, tokenSavings.saved)
  return (
    <StatCard title="Tokens saved">
      <div className="flex items-baseline gap-1">
        <span className="font-extrabold text-3xl text-ink tabular-nums">
          {formatTokens(saved)}
        </span>
        <span className="text-ink-2 text-sm">tokens</span>
      </div>
      <p className="text-ink-2 text-sm">
        <span className="tabular-nums">
          {formatTokens(tokenSavings.otherBrowsers)}
        </span>{' '}
        tokens in other browsers
      </p>
    </StatCard>
  )
}

/** How often the skill runs end to end without a tool error, color-coded. */
function SuccessRateCard({ skill }: { skill: Skill }) {
  const { hasRuns, percent, colorClass } = successRate(
    skill.cleanRunCount,
    skill.runCount,
  )
  return (
    <StatCard title="Success rate">
      <div className="flex items-baseline gap-1">
        <span
          className={cn('font-extrabold text-3xl tabular-nums', colorClass)}
        >
          {hasRuns ? `${percent}%` : 'not run'}
        </span>
      </div>
      <p className="text-ink-2 text-sm">
        {skill.cleanRunCount} of {skill.runCount} runs finished without a tool
        error.
      </p>
      {skill.lastRunAt !== undefined && (
        <p className="text-ink-3 text-xs">
          last run {formatRelativeTime(skill.lastRunAt)}
        </p>
      )}
    </StatCard>
  )
}

/** The coding agents the skill is currently linked into. */
function LinkedIntoCard({ skill }: { skill: Skill }) {
  return (
    <StatCard title="Linked into">
      {skill.linkedAgents.length === 0 ? (
        <p className="text-ink-3 text-sm">Not linked into any agent.</p>
      ) : (
        <div className="flex flex-wrap gap-1.5">
          {skill.linkedAgents.map((agent) => (
            <span
              key={agent}
              className="rounded-9 bg-card-tint px-2 py-1 font-medium text-[12px] text-ledger-ink-2"
            >
              {agentLabel(agent)}
            </span>
          ))}
        </div>
      )}
    </StatCard>
  )
}

const CELL_PADDING = 'px-2 py-2.5 first:pl-4 last:pr-4'

function RunHistory({ runs }: { runs: SkillRun[] }) {
  return (
    <section className="flex flex-col gap-2">
      <h2 className="font-semibold text-ink text-sm">Run history</h2>
      {runs.length === 0 ? (
        <div className="rounded-9 border border-ledger-border bg-card px-6 py-8 text-center text-ink-2 text-sm">
          This task has not run yet.
        </div>
      ) : (
        <div className="overflow-clip rounded-9 border border-ledger-border bg-card">
          <Table className="table-fixed">
            <TableHeader>
              <TableRow className="border-ledger-border border-b bg-ledger-head hover:bg-ledger-head">
                <TableHead
                  className={cn(
                    CELL_PADDING,
                    'h-auto w-16 font-medium text-[12px] text-ledger-head-ink',
                  )}
                >
                  Run
                </TableHead>
                <TableHead
                  className={cn(
                    CELL_PADDING,
                    'h-auto font-medium text-[12px] text-ledger-head-ink',
                  )}
                >
                  Agent
                </TableHead>
                <TableHead
                  className={cn(
                    CELL_PADDING,
                    'h-auto w-24 text-right font-medium text-[12px] text-ledger-head-ink',
                  )}
                >
                  Tokens
                </TableHead>
                <TableHead
                  className={cn(
                    CELL_PADDING,
                    'h-auto w-20 text-right font-medium text-[12px] text-ledger-head-ink',
                  )}
                >
                  Tools
                </TableHead>
                <TableHead
                  className={cn(
                    CELL_PADDING,
                    'h-auto w-40 font-medium text-[12px] text-ledger-head-ink',
                  )}
                >
                  Result
                </TableHead>
                <TableHead
                  className={cn(
                    CELL_PADDING,
                    'h-auto w-24 text-right font-medium text-[12px] text-ledger-head-ink',
                  )}
                >
                  When
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {runs.map((run) => (
                <TableRow
                  key={run.id}
                  className="border-ledger-divider border-b hover:bg-card-tint"
                >
                  <TableCell
                    className={cn(CELL_PADDING, 'text-[13px] text-ink')}
                  >
                    #{run.runNumber}
                  </TableCell>
                  <TableCell
                    className={cn(
                      CELL_PADDING,
                      'truncate text-[13px] text-ledger-ink-2',
                    )}
                  >
                    {agentLabel(run.agentId)}
                  </TableCell>
                  <TableCell
                    className={cn(
                      CELL_PADDING,
                      'text-right text-[13px] text-ink',
                    )}
                  >
                    {run.tokens === undefined
                      ? 'not run'
                      : formatTokens(run.tokens)}
                  </TableCell>
                  <TableCell
                    className={cn(
                      CELL_PADDING,
                      'text-right text-[13px] text-ledger-ink-2',
                    )}
                  >
                    {run.toolCount ?? 0}
                  </TableCell>
                  <TableCell className={cn(CELL_PADDING, 'text-[13px]')}>
                    {run.clean ? (
                      <span className="inline-flex items-center gap-1 text-green">
                        <Check className="size-3.5" />
                        clean
                      </span>
                    ) : (
                      <span className="truncate text-red">
                        {run.erroredTool ?? 'error'}
                      </span>
                    )}
                  </TableCell>
                  <TableCell
                    className={cn(
                      CELL_PADDING,
                      'text-right text-[13px] text-ink-3',
                    )}
                  >
                    {formatRelativeTime(run.createdAt)}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      )}
    </section>
  )
}

function Notice({ children }: { children: ReactNode }) {
  return (
    <div className="rounded-9 border border-ledger-border bg-card px-6 py-10 text-center text-ink-2 text-sm">
      {children}
    </div>
  )
}

function DetailSkeleton() {
  return (
    <div className="flex flex-col gap-6">
      <div className="h-9 w-48 animate-pulse rounded bg-card-tint" />
      <div className="grid grid-cols-1 gap-4 md:grid-cols-3">
        {['c1', 'c2', 'c3'].map((id) => (
          <div
            key={id}
            className="h-32 animate-pulse rounded-9 border border-ledger-border bg-card-tint"
          />
        ))}
      </div>
      <div className="h-64 animate-pulse rounded-9 border border-ledger-border bg-card-tint" />
    </div>
  )
}
