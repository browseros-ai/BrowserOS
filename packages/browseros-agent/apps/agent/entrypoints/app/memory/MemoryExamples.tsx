import { MessageSquare, Send } from 'lucide-react'
import type { FC } from 'react'
import { useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Textarea } from '@/components/ui/textarea'
import { openSidePanelWithSearch } from '@/lib/messaging/sidepanel/openSidepanelWithSearch'

interface Example {
  label: string
  query: string
}

const MEMORY_EXAMPLES: Example[] = [
  {
    label: 'Introduce yourself',
    query:
      'My name is [name] and I work as a [role] at [company]. Save this to your core memory.',
  },
  {
    label: 'Learn from my bookmarks',
    query:
      'Look through my bookmarks and figure out what topics and interests matter to me. Save the key themes to your core memory.',
  },
  {
    label: 'Learn my habits',
    query:
      'Go through my recent browsing history and figure out what tools and sites I rely on, what topics I keep coming back to, and what my day-to-day looks like. Save what you learn about me to core memory.',
  },
  {
    label: 'Know my open tabs',
    query:
      "Look at my open tabs right now and figure out what I'm working on. Save the relevant context to core memory.",
  },
  {
    label: 'Review memories',
    query:
      'Read your core memories and tell me what you know about me. Is anything outdated?',
  },
]

export const MemoryExamples: FC = () => {
  const [editingQuery, setEditingQuery] = useState('')
  const [dialogOpen, setDialogOpen] = useState(false)

  const handleTryIt = (query: string) => {
    setEditingQuery(query)
    setDialogOpen(true)
  }

  const handleSend = () => {
    if (!editingQuery.trim()) return
    openSidePanelWithSearch('open', {
      query: editingQuery.trim(),
      mode: 'agent',
    })
    setDialogOpen(false)
  }

  return (
    <div className="space-y-3">
      <div>
        <h3 className="font-medium text-sm">Teach your Sup about you</h3>
        <p className="mt-1 text-muted-foreground text-xs">
          Use these prompts to help your Sup learn. Edit the message before
          sending.
        </p>
      </div>

      <div className="grid gap-2">
        {MEMORY_EXAMPLES.map((example) => (
          <div
            key={example.label}
            className="flex flex-col gap-3 rounded-lg border border-border bg-card p-3 transition-colors hover:bg-muted/50 sm:flex-row sm:items-center sm:justify-between"
          >
            <div className="min-w-0 flex-1">
              <p className="font-medium text-sm">{example.label}</p>
              <p className="mt-0.5 line-clamp-2 text-muted-foreground text-xs leading-relaxed">
                {example.query}
              </p>
            </div>
            <Button
              variant="outline"
              size="sm"
              className="shrink-0 gap-1.5 self-start sm:self-center"
              onClick={() => handleTryIt(example.query)}
            >
              <MessageSquare className="h-3.5 w-3.5" />
              Try it
            </Button>
          </div>
        ))}
      </div>

      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Edit message</DialogTitle>
            <DialogDescription>
              Customize the prompt before sending it to your Sup.
            </DialogDescription>
          </DialogHeader>
          <Textarea
            value={editingQuery}
            onChange={(e) => setEditingQuery(e.target.value)}
            rows={4}
            className="resize-none"
          />
          <DialogFooter>
            <Button variant="outline" onClick={() => setDialogOpen(false)}>
              Cancel
            </Button>
            <Button
              onClick={handleSend}
              disabled={!editingQuery.trim()}
              className="gap-1.5"
            >
              <Send className="h-3.5 w-3.5" />
              Send
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
