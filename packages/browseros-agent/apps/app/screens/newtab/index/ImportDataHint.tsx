import { Upload, X } from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Checkbox } from '@/components/ui/checkbox'
import { importHintDismissedAtStorage } from '@/lib/onboarding/onboardingStorage'

const importSettingsURL = 'chrome://settings/importData'

export const ImportDataHint = () => {
  const [dismissed, setDismissed] = useState(false)
  const [dontAskAgain, setDontAskAgain] = useState(false)

  const handleDismiss = async () => {
    setDismissed(true)
    if (dontAskAgain) {
      await importHintDismissedAtStorage.setValue(
        Date.now() + 100 * 365 * 24 * 60 * 60 * 1000,
      )
    } else {
      await importHintDismissedAtStorage.setValue(Date.now())
    }
  }

  const handleImport = () => {
    chrome.tabs.create({ url: importSettingsURL })
    handleDismiss()
  }

  return (
    <AnimatePresence>
      {!dismissed && (
        <motion.div
          className="fixed right-4 bottom-4 z-50"
          initial={{ opacity: 0, x: 100 }}
          animate={{ opacity: 1, x: 0 }}
          exit={{ opacity: 0, x: 100 }}
          transition={{ type: 'spring', damping: 25, stiffness: 300 }}
        >
          <Card className="w-80 gap-0 py-4">
            <CardHeader className="gap-3">
              <div className="flex items-start justify-between">
                <div className="flex items-center gap-2">
                  <Upload className="size-5 text-muted-foreground" />
                  <CardTitle className="text-base">Import your data</CardTitle>
                </div>
                <Button
                  variant="ghost"
                  size="icon"
                  className="size-7 shrink-0"
                  onClick={handleDismiss}
                >
                  <X className="size-4" />
                </Button>
              </div>
              <CardDescription>
                Bring bookmarks, history, and passwords from Chrome.
              </CardDescription>
              <label
                htmlFor="import-dont-ask-again"
                className="flex items-center gap-2 text-muted-foreground text-sm"
              >
                <Checkbox
                  id="import-dont-ask-again"
                  checked={dontAskAgain}
                  onCheckedChange={(checked) =>
                    setDontAskAgain(checked === true)
                  }
                />
                Don't show this again
              </label>
              <Button className="w-full" onClick={handleImport}>
                <Upload className="size-4" />
                Open Import Settings
              </Button>
            </CardHeader>
          </Card>
        </motion.div>
      )}
    </AnimatePresence>
  )
}
