/**
 * Self-hosted variable + static fonts. Importing these here ships the
 * font files inside the extension bundle so the cockpit does not depend
 * on Google's CDN (chrome-extension:// + googleapis.com pairing would
 * either need broad host_permissions or a noticeable FOUC on cold
 * loads). Variable Schibsted Grotesk + JetBrains Mono carry the full
 * weight range in a single file; Newsreader is imported as the italic
 * variants only because the cockpit uses it for the hero accent.
 */
import '@fontsource-variable/schibsted-grotesk'
import '@fontsource/newsreader/400-italic.css'
import '@fontsource/newsreader/500-italic.css'
import '@fontsource-variable/jetbrains-mono'

import { QueryClientProvider } from '@tanstack/react-query'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { TooltipProvider } from '@/components/ui/tooltip'
import { queryClient } from '@/modules/api/queryClient'
import { App } from './App'
import './styles.css'

const root = document.getElementById('root')
if (!root) throw new Error('Root element not found')

createRoot(root).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <TooltipProvider delay={0}>
        <App />
      </TooltipProvider>
    </QueryClientProvider>
  </StrictMode>,
)
