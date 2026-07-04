import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { BrowserRouter } from 'react-router'
import { App } from './App'
import './styles.css'

// Track the OS color preference and mirror it onto <html> as `.dark`, which
// flips the palette variables and the shadcn `dark:` variants. Applied as early
// as the single inlined bundle allows — before React mounts, so app content
// never renders on the wrong theme — and kept live via a change listener so
// flipping OS appearance mid-onboarding updates immediately. The bundle is
// deferred, so dark-mode users may still see a brief first-paint flash of the
// light body gradient; a blocking inline <script> would avoid it but is
// disallowed by the strict WebUI/extension CSP (the same reason claw-app
// extracts its theme script to a module chunk, which this app's single-bundle
// chromium contract also rules out). Onboarding is a first-run flow with no
// theme toggle and the host bridge carries no theme signal, so
// `prefers-color-scheme` is the source of truth.
if (
  typeof document !== 'undefined' &&
  typeof window !== 'undefined' &&
  typeof window.matchMedia === 'function'
) {
  const media = window.matchMedia('(prefers-color-scheme: dark)')
  const applyTheme = (dark: boolean) =>
    document.documentElement.classList.toggle('dark', dark)
  applyTheme(media.matches)
  media.addEventListener('change', (event) => applyTheme(event.matches))
}

const root = document.getElementById('root')
if (!root) throw new Error('Root element not found')

createRoot(root).render(
  <StrictMode>
    <BrowserRouter>
      <App />
    </BrowserRouter>
  </StrictMode>,
)
