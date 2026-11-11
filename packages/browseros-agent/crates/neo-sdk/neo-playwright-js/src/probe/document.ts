/**
 * The document as a whole. `page.title()` and `page.content()` read it here rather than in
 * the main world, so that every field value the serialization reads comes through the seal
 * (`value="••••"`). `page.setContent` replaces it here too, as upstream does from its utility
 * world. A failed `expect` that found nothing reads the body's aria snapshot here, as upstream
 * reads it from its utility world.
 */
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
  const text =
    request.part === 'title'
      ? document.title
      : request.part === 'expectSnapshot'
        ? document.body
          ? injected.ariaSnapshotForExpectFailure(document.body, {
              mode: 'default',
            })
          : ''
        : content(document)
  return { kind: 'documentText', text }
}

/** Upstream's `frame.content()`: the doctype, then the root element's markup. */
function content(document: Document): string {
  const doctype = document.doctype ? doctypeMarkup(document.doctype) : ''
  return doctype + (document.documentElement?.outerHTML ?? '')
}

/**
 * The doctype as XMLSerializer prints it (DOM Parsing, "serializing a DocumentType"). Upstream
 * calls XMLSerializer, which the seal deletes from this world.
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
 *
 * document.open() also removes every event listener of the document, the seal's journal
 * listeners among them; `watch` adds them back.
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
    world.seal.watch(document)
  }
  return { kind: 'replaced' }
}
