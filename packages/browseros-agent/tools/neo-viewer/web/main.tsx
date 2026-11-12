/**
 * The page's entry. `/` is the designed screen (Viewer C v2); `/?plain` keeps PR B's plain run
 * list, on the same API, for debugging. The viewer serves one page, so the choice is made here
 * rather than by a server route.
 */

import '@fontsource/ibm-plex-sans/latin-400.css'
import '@fontsource/ibm-plex-sans/latin-500.css'
import '@fontsource/ibm-plex-sans/latin-600.css'
import '@fontsource-variable/jetbrains-mono/wght.css'
import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { App } from './App'
import { ViewerScreen } from './screen/ViewerScreen'

const plain = new URLSearchParams(window.location.search).has('plain')
const root = document.getElementById('root')
if (root) {
  createRoot(root).render(
    <StrictMode>{plain ? <App /> : <ViewerScreen />}</StrictMode>,
  )
}
