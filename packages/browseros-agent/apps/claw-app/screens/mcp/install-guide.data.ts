/**
 * The Claude Desktop install walkthrough, as data. The step rail, the
 * counter, the Back/Next bounds, and the tests all derive from this one
 * list, so reordering or adding a step is a single edit here.
 */

export const EXTENSION_DOWNLOAD_URL =
  'https://github.com/browseros-ai/browserclaw-claude-desktop/releases/download/v0.5.0/browseros-neo-0.5.0.mcpb'

export const EXTENSION_RELEASES_URL =
  'https://github.com/browseros-ai/browserclaw-claude-desktop/releases'

/** Derived so the file named in the picker step can never drift from the download. */
export const MCPB_FILENAME = EXTENSION_DOWNLOAD_URL.slice(
  EXTENSION_DOWNLOAD_URL.lastIndexOf('/') + 1,
)

export const COWORK_REQUIREMENT_LINE =
  'To use BrowserOS neo with Claude Cowork, you need to install this extension.'

/**
 * Screenshots live in the `browserclaw` R2 bucket rather than the extension
 * bundle so a Claude Desktop UI change can be corrected by re-uploading the
 * image, with no extension release. Masters are in `~/Desktop/
 * browseros-neo-install-screenshots` and in git history at commit 1d1c794ba.
 */
const IMAGE_BASE = 'https://pub-c94be9094f01420f9166e717fbd4a20d.r2.dev'

export interface InstallStepImage {
  src: string
  alt: string
}

export interface InstallStep {
  id: string
  /** Rail label — 2-3 words. */
  title: string
  body: string
  image?: InstallStepImage
  /** The one step that swaps its screenshot slot for the download CTA. */
  kind?: 'download'
}

export const INSTALL_STEPS: readonly InstallStep[] = [
  {
    id: 'open-settings',
    title: 'Open Settings',
    body: 'In Claude Desktop, click your account at the bottom-left of the sidebar, then choose Settings.',
    image: {
      src: `${IMAGE_BASE}/step-settings.webp`,
      alt: 'Claude Desktop with the account menu open at the bottom-left and the Settings item highlighted',
    },
  },
  {
    id: 'open-extensions',
    title: 'Extensions',
    body: 'In the Settings sidebar, scroll to Desktop app and click Extensions. Then click Advanced settings on the right.',
    image: {
      src: `${IMAGE_BASE}/step-extensions.webp`,
      alt: 'Claude Desktop Settings with Extensions selected under Desktop app and the Advanced settings button highlighted',
    },
  },
  {
    id: 'download',
    title: 'Download',
    kind: 'download',
    body: `Download the BrowserOS neo extension. It saves to your Downloads folder as ${MCPB_FILENAME}.`,
  },
  {
    id: 'install-extension',
    title: 'Install Extension',
    body: 'On the Advanced settings page, scroll down to Extension Developer and click Install Extension.',
    image: {
      src: `${IMAGE_BASE}/step-install-extension.webp`,
      alt: 'The Extension Settings page in Claude Desktop with the Install Extension button highlighted under Extension Developer',
    },
  },
  {
    id: 'choose-file',
    title: 'Pick the file',
    body: `In the file picker, select the ${MCPB_FILENAME} you just downloaded.`,
    image: {
      src: `${IMAGE_BASE}/step-choose-file.webp`,
      alt: 'A macOS file picker open over Claude Desktop Settings with the downloaded .mcpb extension file listed',
    },
  },
  {
    id: 'confirm-install',
    title: 'Click Install',
    body: 'Claude shows the BrowserOS neo extension details. Click Install to finish — Claude can now drive BrowserOS neo.',
    image: {
      src: `${IMAGE_BASE}/step-confirm-install.webp`,
      alt: "Claude Desktop's BrowserOS neo extension details sheet with the Install button in the top right",
    },
  },
]
