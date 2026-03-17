import {
  AlertCircle,
  HelpCircle,
  Pencil,
  Plus,
  Trash2,
  Wand2,
} from 'lucide-react'
import { type FC, useEffect, useState } from 'react'
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
} from '@/components/ui/alert-dialog'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { MarkdownEditor } from '@/components/ui/MarkdownEditor'
import { Switch } from '@/components/ui/switch'
import { Textarea } from '@/components/ui/textarea'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { skillsHelpUrl } from '@/lib/constants/productUrls'
import { type SkillDetail, type SkillMeta, useSkills } from './useSkills'

const loadingSkillCards = [
  'loading-a',
  'loading-b',
  'loading-c',
  'loading-d',
  'loading-e',
  'loading-f',
]

export const SkillsPage: FC = () => {
  const {
    skills,
    isLoading,
    error,
    refetch,
    createSkill,
    updateSkill,
    deleteSkill,
    fetchSkillDetail,
  } = useSkills()

  const [isDialogOpen, setIsDialogOpen] = useState(false)
  const [editingSkill, setEditingSkill] = useState<SkillDetail | null>(null)
  const [skillToDelete, setSkillToDelete] = useState<SkillMeta | null>(null)

  const enabledCount = skills.filter((skill) => skill.enabled).length

  const handleCreate = () => {
    setEditingSkill(null)
    setIsDialogOpen(true)
  }

  const handleEdit = async (skill: SkillMeta) => {
    try {
      const detail = await fetchSkillDetail(skill.id)
      setEditingSkill(detail)
      setIsDialogOpen(true)
    } catch {
      toast.error('Failed to load skill details')
    }
  }

  const handleToggle = async (skill: SkillMeta, enabled: boolean) => {
    try {
      await updateSkill(skill.id, { enabled })
    } catch {
      toast.error('Failed to toggle skill')
    }
  }

  const handleDelete = async () => {
    if (!skillToDelete) return
    try {
      await deleteSkill(skillToDelete.id)
      toast.success(`Deleted "${skillToDelete.name}"`)
    } catch {
      toast.error('Failed to delete skill')
    }
    setSkillToDelete(null)
  }

  return (
    <div className="fade-in slide-in-from-bottom-5 animate-in space-y-6 duration-500">
      <SkillsHeader
        skillCount={skills.length}
        enabledCount={enabledCount}
        onCreateClick={handleCreate}
      />

      {isLoading ? <SkillsLoadingState /> : null}

      {!isLoading && error ? (
        <SkillsErrorState onRetry={() => void refetch()} />
      ) : null}

      {!isLoading && !error && skills.length === 0 ? (
        <EmptyState onCreateClick={handleCreate} />
      ) : null}

      {!isLoading && !error && skills.length > 0 ? (
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-3">
          {skills.map((skill) => (
            <SkillCard
              key={skill.id}
              skill={skill}
              onEdit={() => handleEdit(skill)}
              onDelete={() => setSkillToDelete(skill)}
              onToggle={(enabled) => handleToggle(skill, enabled)}
            />
          ))}
        </div>
      ) : null}

      <SkillDialog
        open={isDialogOpen}
        onOpenChange={setIsDialogOpen}
        editingSkill={editingSkill}
        onSave={async (data) => {
          try {
            if (editingSkill) {
              await updateSkill(editingSkill.id, data)
              toast.success('Skill updated')
            } else {
              await createSkill(data)
              toast.success('Skill created')
            }
            setIsDialogOpen(false)
          } catch (err) {
            toast.error(err instanceof Error ? err.message : 'Failed to save')
          }
        }}
      />

      <AlertDialog
        open={!!skillToDelete}
        onOpenChange={(open) => !open && setSkillToDelete(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete Skill</AlertDialogTitle>
            <AlertDialogDescription>
              Are you sure you want to delete "{skillToDelete?.name}"? This
              action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={handleDelete}>Delete</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

const SkillsHeader: FC<{
  skillCount: number
  enabledCount: number
  onCreateClick: () => void
}> = ({ skillCount, enabledCount, onCreateClick }) => {
  const skillLabel = `${skillCount} skill${skillCount === 1 ? '' : 's'}`
  const enabledLabel = `${enabledCount} enabled`

  return (
    <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
      <div>
        <div className="flex items-center gap-2">
          <h1 className="font-semibold text-2xl tracking-tight">Skills</h1>
          <TooltipProvider delayDuration={0}>
            <Tooltip>
              <TooltipTrigger asChild>
                <a
                  href={skillsHelpUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="rounded-full p-1 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                >
                  <HelpCircle className="h-4 w-4" />
                </a>
              </TooltipTrigger>
              <TooltipContent>Learn more about skills</TooltipContent>
            </Tooltip>
          </TooltipProvider>
        </div>
        <p className="text-muted-foreground text-sm">
          Define reusable instructions that extend how your agent responds.
        </p>
        <p className="mt-1 text-muted-foreground text-xs">
          {skillLabel} • {enabledLabel}
        </p>
      </div>
      <Button onClick={onCreateClick} size="sm" className="shrink-0">
        <Plus className="mr-1.5 size-4" />
        New Skill
      </Button>
    </div>
  )
}

const SkillsLoadingState: FC = () => (
  <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-3">
    {loadingSkillCards.map((cardKey) => (
      <Card key={cardKey} className="h-full py-0">
        <CardContent className="space-y-4 p-5">
          <div className="flex items-start justify-between gap-3">
            <div className="size-10 animate-pulse rounded-xl bg-muted" />
            <div className="h-6 w-11 animate-pulse rounded-full bg-muted" />
          </div>
          <div className="space-y-2">
            <div className="h-4 w-28 animate-pulse rounded bg-muted" />
            <div className="h-4 w-full animate-pulse rounded bg-muted" />
            <div className="h-4 w-4/5 animate-pulse rounded bg-muted" />
          </div>
          <div className="h-8 w-16 animate-pulse rounded bg-muted" />
        </CardContent>
      </Card>
    ))}
  </div>
)

const SkillsErrorState: FC<{ onRetry: () => void }> = ({ onRetry }) => (
  <Card className="border-destructive/20 bg-destructive/5 py-0">
    <CardContent className="flex flex-col gap-4 p-5 sm:flex-row sm:items-center sm:justify-between">
      <div className="flex items-start gap-3">
        <div className="flex size-10 shrink-0 items-center justify-center rounded-xl bg-destructive/10 text-destructive">
          <AlertCircle className="size-4" />
        </div>
        <div className="space-y-1">
          <h2 className="font-semibold">Couldn&apos;t load skills</h2>
          <p className="text-destructive/80 text-sm">
            Check that the local agent services are running, then retry.
          </p>
        </div>
      </div>
      <Button variant="outline" onClick={onRetry}>
        Retry
      </Button>
    </CardContent>
  </Card>
)

const EmptyState: FC<{ onCreateClick: () => void }> = ({ onCreateClick }) => (
  <Card className="border-dashed py-0">
    <CardContent className="flex flex-col items-center justify-center py-14 text-center">
      <div className="mb-4 flex size-12 items-center justify-center rounded-2xl bg-[var(--accent-orange)]/10 text-[var(--accent-orange)]">
        <Wand2 className="size-5" />
      </div>
      <h3 className="mb-1 font-medium text-lg">No skills yet</h3>
      <p className="mb-5 max-w-sm text-muted-foreground text-sm leading-6">
        Skills teach your agent how to handle repeatable tasks like research,
        extraction, and structured workflows.
      </p>
      <Button onClick={onCreateClick} size="sm">
        <Plus className="mr-1.5 size-4" />
        Create your first skill
      </Button>
    </CardContent>
  </Card>
)

const SkillCard: FC<{
  skill: SkillMeta
  onEdit: () => void
  onDelete: () => void
  onToggle: (enabled: boolean) => void
}> = ({ skill, onEdit, onDelete, onToggle }) => (
  <Card className="h-full py-0 shadow-sm">
    <CardContent className="flex h-full flex-col p-4">
      <div className="flex items-start justify-between gap-3">
        <h2 className="font-semibold text-sm leading-5">{skill.name}</h2>
        <Switch
          checked={skill.enabled}
          onCheckedChange={onToggle}
          aria-label={`Toggle ${skill.name}`}
        />
      </div>

      <div className="mt-3 flex-1">
        <p className="line-clamp-3 text-muted-foreground text-sm leading-5">
          {skill.description}
        </p>
      </div>

      <div className="mt-3 flex items-center justify-between gap-2">
        <Button
          variant="ghost"
          size="sm"
          onClick={onEdit}
          className="-ml-2 h-7 px-2 text-muted-foreground hover:bg-transparent hover:text-foreground"
        >
          <Pencil className="size-3.5" />
          Edit
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          onClick={onDelete}
          className="size-7 text-muted-foreground hover:bg-transparent hover:text-destructive"
          aria-label={`Delete ${skill.name}`}
        >
          <Trash2 className="size-4" />
        </Button>
      </div>
    </CardContent>
  </Card>
)

const SkillDialog: FC<{
  open: boolean
  onOpenChange: (open: boolean) => void
  editingSkill: SkillDetail | null
  onSave: (data: {
    name: string
    description: string
    content: string
  }) => Promise<void>
}> = ({ open, onOpenChange, editingSkill, onSave }) => {
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [content, setContent] = useState('')
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    setSaving(false)
    if (!open) return
    setName(editingSkill?.name ?? '')
    setDescription(editingSkill?.description ?? '')
    setContent(editingSkill?.content ?? '')
  }, [editingSkill, open])

  const isValid =
    name.trim().length > 0 &&
    description.trim().length > 0 &&
    content.trim().length > 0

  const handleSubmit = async () => {
    if (!isValid || saving) return
    setSaving(true)
    try {
      await onSave({
        name: name.trim(),
        description: description.trim(),
        content,
      })
    } finally {
      setSaving(false)
    }
  }

  const handleContentKeyDown: React.KeyboardEventHandler<HTMLDivElement> = (
    event,
  ) => {
    if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
      event.preventDefault()
      void handleSubmit()
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[90vh] flex-col gap-0 overflow-hidden p-0 sm:max-w-5xl">
        <DialogHeader className="border-b px-6 py-5">
          <DialogTitle>
            {editingSkill ? 'Edit Skill' : 'Create Skill'}
          </DialogTitle>
          <DialogDescription>
            {editingSkill
              ? 'Refine when the agent should use this skill and how it should execute it.'
              : 'Define a reusable instruction set your agent can apply when a request matches.'}
          </DialogDescription>
        </DialogHeader>

        <div className="grid min-h-0 flex-1 overflow-y-auto lg:grid-cols-[280px_minmax(0,1fr)] lg:overflow-hidden">
          <div className="space-y-5 border-b bg-muted/20 px-6 py-5 lg:border-r lg:border-b-0">
            <div className="space-y-2">
              <Label htmlFor="skill-name">Name</Label>
              <Input
                id="skill-name"
                placeholder="e.g., Read Later"
                value={name}
                onChange={(event) => setName(event.target.value)}
                maxLength={100}
              />
              <p className="text-muted-foreground text-xs leading-5">
                Keep it short and recognizable in the skills list.
              </p>
            </div>

            <div className="space-y-2">
              <Label htmlFor="skill-description">Description</Label>
              <Textarea
                id="skill-description"
                placeholder="Describe when the agent should use this skill."
                value={description}
                onChange={(event) => setDescription(event.target.value)}
                maxLength={500}
                className="min-h-28 resize-none bg-background"
              />
              <p className="text-muted-foreground text-xs leading-5">
                This is the trigger summary the agent uses to pick the skill.
              </p>
            </div>

            <div className="mt-auto rounded-lg border border-border/60 border-dashed bg-muted/30 px-3 py-2.5">
              <p className="font-medium text-muted-foreground text-xs">Tip</p>
              <ul className="mt-1.5 list-disc space-y-1 pl-4 text-muted-foreground text-xs leading-5">
                <li>List the ordered steps the agent should follow.</li>
                <li>Close with the output or formatting you expect back.</li>
              </ul>
            </div>
          </div>

          <div className="flex min-h-0 flex-col px-6 py-5">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <Label htmlFor="skill-content">Instructions (Markdown)</Label>
              <Badge variant="outline" className="border-border bg-background">
                {content.length} characters
              </Badge>
            </div>

            <MarkdownEditor
              id="skill-content"
              value={content}
              onChange={setContent}
              onKeyDown={handleContentKeyDown}
              placeholder="Write instructions for the agent. Use markdown for structure."
              className="mt-4 min-h-[320px] flex-1 overflow-y-auto text-sm"
            />
          </div>
        </div>

        <div className="flex flex-col gap-3 border-t px-6 py-4 sm:flex-row sm:items-center sm:justify-between">
          <p className="text-muted-foreground text-xs">
            Saved locally and available to your agent immediately.
          </p>
          <div className="flex flex-col-reverse gap-2 sm:flex-row">
            <Button
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={saving}
            >
              Cancel
            </Button>
            <Button onClick={handleSubmit} disabled={!isValid || saving}>
              {saving
                ? 'Saving...'
                : editingSkill
                  ? 'Update Skill'
                  : 'Create Skill'}
            </Button>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}
