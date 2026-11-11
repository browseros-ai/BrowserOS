// Generated from neo-playwright's op table (`Op::all()`). Do not edit: change the
// op's family file in Rust, then run
// `cargo test -p neo-playwright --test ops_generated`, which rewrites this file.

/**
 * Every operation the prelude can invoke, in Rust's registration order, with what the
 * host's dispatcher knows about it: whether it may change the page, which input field
 * holds the page handle, and whether it is the background event pump.
 */
export const OPS = {
  'locator.click': {
    effect: 'write',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.dblclick': {
    effect: 'write',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.hover': {
    effect: 'write',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.check': {
    effect: 'write',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.uncheck': {
    effect: 'write',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.dragTo': {
    effect: 'write',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.fill': {
    effect: 'write',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.press': {
    effect: 'write',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.pressSequentially': {
    effect: 'write',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.selectOption': {
    effect: 'write',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.selectText': {
    effect: 'write',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.focus': {
    effect: 'write',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.blur': {
    effect: 'write',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.setInputFiles': {
    effect: 'write',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.dispatchEvent': {
    effect: 'write',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.scrollIntoViewIfNeeded': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.textContent': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.innerText': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.innerHTML': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.getAttribute': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.inputValue': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.isEnabled': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.isDisabled': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.isEditable': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.isChecked': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.isVisible': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.isHidden': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.count': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.allTextContents': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.allInnerTexts': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.boundingBox': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.waitFor': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.elementHandle': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.elementHandles': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.normalize': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'page.ariaSnapshot': {
    effect: 'read',
    target: 'page',
    lifetime: 'operation',
  },
  'locator.ariaSnapshot': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'expect.locator': {
    effect: 'read',
    target: 'target',
    lifetime: 'operation',
  },
  'expect.page': {
    effect: 'read',
    target: 'page',
    lifetime: 'operation',
  },
  'keyboard.down': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'keyboard.up': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'keyboard.press': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'keyboard.type': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'keyboard.insertText': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'mouse.move': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'mouse.down': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'mouse.up': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'mouse.click': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'mouse.wheel': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'page.goto': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'page.reload': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'page.goBack': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'page.goForward': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'page.waitForNavigation': {
    effect: 'read',
    target: 'page',
    lifetime: 'operation',
  },
  'page.waitForURL': {
    effect: 'read',
    target: 'page',
    lifetime: 'operation',
  },
  'page.waitForLoadState': {
    effect: 'read',
    target: 'page',
    lifetime: 'operation',
  },
  'page.title': {
    effect: 'read',
    target: 'page',
    lifetime: 'operation',
  },
  'page.content': {
    effect: 'read',
    target: 'page',
    lifetime: 'operation',
  },
  'page.setContent': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'page.addScriptTag': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'page.addStyleTag': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'page.close': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'page.opener': {
    effect: 'read',
    target: 'page',
    lifetime: 'operation',
  },
  'page.frames': {
    effect: 'read',
    target: 'page',
    lifetime: 'operation',
  },
  'page.evaluate': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'locator.evaluate': {
    effect: 'write',
    target: 'target',
    lifetime: 'operation',
  },
  'locator.evaluateAll': {
    effect: 'write',
    target: 'target',
    lifetime: 'operation',
  },
  'page.waitForFunction': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'locator.waitForFunction': {
    effect: 'write',
    target: 'target',
    lifetime: 'operation',
  },
  'page.screenshot': {
    effect: 'read',
    target: 'page',
    lifetime: 'operation',
  },
  'page.pdf': {
    effect: 'read',
    target: 'page',
    lifetime: 'operation',
  },
  'context.pages': {
    effect: 'read',
    target: null,
    lifetime: 'operation',
  },
  'context.newPage': {
    effect: 'write',
    target: null,
    lifetime: 'operation',
  },
  'events.listen': {
    effect: 'read',
    target: null,
    lifetime: 'operation',
  },
  'events.next': {
    effect: 'read',
    target: null,
    lifetime: 'background',
  },
  'events.wait': {
    effect: 'read',
    target: null,
    lifetime: 'operation',
  },
  'events.interceptFileChooser': {
    effect: 'read',
    target: null,
    lifetime: 'operation',
  },
  'dialog.accept': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'dialog.dismiss': {
    effect: 'write',
    target: 'page',
    lifetime: 'operation',
  },
  'download.path': {
    effect: 'read',
    target: null,
    lifetime: 'operation',
  },
  'download.saveAs': {
    effect: 'write',
    target: null,
    lifetime: 'operation',
  },
} as const

/** A name `__neo.invoke` accepts for a Playwright operation. */
export type OpName = keyof typeof OPS
