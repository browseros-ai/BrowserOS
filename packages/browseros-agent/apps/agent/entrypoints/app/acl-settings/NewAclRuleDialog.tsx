import type { AclRule } from '@browseros/shared/types/acl'
import { type FC, useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'

interface NewAclRuleDialogProps {
  onSave: (rule: AclRule) => void
  children: React.ReactNode
}

export const NewAclRuleDialog: FC<NewAclRuleDialogProps> = ({
  onSave,
  children,
}) => {
  const [open, setOpen] = useState(false)
  const [sitePattern, setSitePattern] = useState('')
  const [intent, setIntent] = useState('')

  const reset = () => {
    setSitePattern('')
    setIntent('')
  }

  const handleSave = () => {
    if (!sitePattern.trim() || !intent.trim()) return
    onSave({
      id: crypto.randomUUID(),
      sitePattern: sitePattern.trim(),
      description: intent.trim(),
      enabled: true,
    })
    reset()
    setOpen(false)
  }

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>{children}</DialogTrigger>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Add ACL Rule</DialogTitle>
        </DialogHeader>
        <div className="flex flex-col gap-4 py-4">
          <div className="flex flex-col gap-2">
            <Label htmlFor="site-pattern">
              Domain <span className="text-destructive">*</span>
            </Label>
            <Input
              id="site-pattern"
              placeholder="amazon.com"
              value={sitePattern}
              onChange={(e) => setSitePattern(e.target.value)}
            />
            <p className="text-muted-foreground text-xs">
              Matches the domain and all subdomains.
            </p>
          </div>
          <div className="flex flex-col gap-2">
            <Label htmlFor="intent">
              What should BrowserOS block?{' '}
              <span className="text-destructive">*</span>
            </Label>
            <Input
              id="intent"
              placeholder="Payments and checkout"
              value={intent}
              onChange={(e) => setIntent(e.target.value)}
            />
            <p className="text-muted-foreground text-xs">
              Use plain English. BrowserOS will block matching actions on this
              site.
            </p>
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => setOpen(false)}>
            Cancel
          </Button>
          <Button
            onClick={handleSave}
            disabled={!sitePattern.trim() || !intent.trim()}
          >
            Add Rule
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
