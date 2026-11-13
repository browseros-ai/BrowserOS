/**
 * The document as a whole. `page.title()` and `page.content()` read it here, in the engine's
 * isolated world, and `page.setContent` replaces it here, as upstream does each from its
 * utility world. A failed `expect` that found nothing reads the body's aria snapshot here, as
 * upstream reads it from its utility world.
 */

import { withMarkedFieldsMasked } from './private-field-values'
import type {
  DocumentRequest,
  DocumentText,
  Replaced,
  SetContentRequest,
} from './protocol'
import type { World } from './world'

export function documentText(
  world: World,
  request: DocumentRequest,
): DocumentText {
  const { injected } = world
  const { document } = injected
  const { body } = document
  const text =
    request.part === 'title'
      ? document.title
      : request.part === 'expectSnapshot'
        ? body
          ? // A box Neo typed a host's secret into prints bullets here too (ruling R44).
            withMarkedFieldsMasked(() =>
              injected.ariaSnapshotForExpectFailure(body, { mode: 'default' }),
            )
          : ''
        : request.includeShadow
          ? // The vendored serializer recursively includes open roots, even non-serializable
            // ones, while closed roots remain inaccessible from the engine's isolated world.
            injected.documentContent(true)
          : content(document)
  return { kind: 'documentText', text }
}

/** Upstream's `frame.content()`: the doctype, then the root element's markup. */
function content(document: Document): string {
  const doctype = document.doctype ? doctypeMarkup(document.doctype) : ''
  return doctype + (document.documentElement?.outerHTML ?? '')
}

/**
 * The doctype as XMLSerializer prints it (DOM Parsing, "serializing a DocumentType"), which is
 * what upstream calls.
 */
function doctypeMarkup(doctype: DocumentType): string {
  let markup = `<!DOCTYPE ${doctype.name}`
  if (doctype.publicId) markup += ` PUBLIC "${doctype.publicId}"`
  else if (doctype.systemId) markup += ' SYSTEM'
  if (doctype.systemId) markup += ` "${doctype.systemId}"`
  return `${markup}>`
}

/**
 * Replaces the document's content with `html` (measured on BrowserOS: the same as
 * Page.setDocumentContent, and Chromium then fires `init`, `DOMContentLoaded` and `load` again
 * under the same loader).
 *
 * document.open() keeps the Document and its window, so this world, its engine and its seq carry
 * on, as upstream's utility-world InjectedScript does: the frame commits no new document, so
 * upstream's frame keeps its seq and the next snapshot's refs keep their prefix (`e5`, not
 * `f1e5`). Every pin and ref from before names an element document.open() removed, so it reads
 * as detached or stale, never as an element of the new content.
 */
export function replaceDocument(
  world: World,
  request: SetContentRequest,
): Replaced {
  const { document } = world.injected
  document.open()
  try {
    document.write(request.html)
  } finally {
    document.close()
  }
  return { kind: 'replaced' }
}
