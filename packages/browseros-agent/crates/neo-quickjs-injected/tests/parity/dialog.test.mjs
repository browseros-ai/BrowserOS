// Dialogs through the pinned Playwright and through Neo, on fresh tests/pages/dialogs.html tabs
// (events.mjs, which disconnects Playwright before Neo's turn: Playwright's server would answer
// Neo's dialogs too). An alert reaches the context's 'dialog' listeners and then the page's, as
// one Dialog that knows its page; accepting it closes it and fires 'dialogclosed' with it, and
// a handled dialog refuses a second answer. prompt and confirm return what the listener chose,
// read back from the page's #result. With no listener the click still resolves, the dialog is
// dismissed, and 'dialogclosed' still fires; removing the last listener brings that back.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs, events.mjs).

import { test } from 'node:test'
import { compareEvents, fixture } from './events.mjs'
import { skip } from './harness.mjs'

const cases = [
  {
    // tests/page/page-dialog.spec.ts "DI-001 should fire", "DI-002 should fire dialogclosed
    // when dialog is accepted"; tests/library/browsercontext-events.spec.ts "BC-037 dialog event
    // should work @smoke", "BC-038 dialogclosed event should work".
    name: "an alert reaches context then page listeners; accept fires 'dialogclosed'",
    path: '/dialogs.html',
    source: `const order = []
    let opened
    let accepted
    page.context().on('dialog', (dialog) => {
      order.push('context dialog')
      opened = dialog
    })
    page.on('dialog', (dialog) => {
      order.push('page dialog' + (dialog === opened ? '' : ' (another object)'))
      accepted = dialog.accept()
    })
    page.context().on('dialogclosed', (dialog) => {
      order.push('context dialogclosed' + (dialog === opened ? '' : ' (another object)'))
    })
    const [closed] = await Promise.all([page.waitForEvent('dialogclosed'), page.click('#alert')])
    return {
      dialog: [opened.type(), opened.message(), opened.defaultValue(), opened.page() === page],
      closedIsOpened: closed === opened,
      order,
      accepted: await settle(accepted),
      result: await page.textContent('#result'),
      acceptAgain: await settle(opened.accept()),
      dismissAgain: await settle(opened.dismiss()),
    }`,
    expected: {
      dialog: ['alert', 'yo', '', true],
      closedIsOpened: true,
      order: ['context dialog', 'page dialog', 'context dialogclosed'],
      accepted: 'undefined',
      result: '"undefined"',
      acceptAgain: { error: 'Error' },
      dismissAgain: { error: 'Error' },
    },
    probe: 'window.__answers',
    effect: ['"undefined"'],
  },
  {
    // page-dialog.spec.ts "DI-005 should allow accepting prompts @smoke", "DI-006 should
    // dismiss the prompt", "DI-007 should accept the confirm prompt", "DI-008 should dismiss
    // the confirm prompt", "DI-003 should fire dialogclosed when dialog is dismissed".
    name: 'prompt and confirm return what the listener chose',
    path: '/dialogs.html',
    source: `const dialogs = []
    let answer
    page.on('dialog', (dialog) => {
      dialogs.push([dialog.type(), dialog.message(), dialog.defaultValue()])
      void answer(dialog)
    })
    const results = {}
    const click = async (name, selector, respond) => {
      answer = respond
      await page.click(selector)
      results[name] = await page.textContent('#result')
    }
    await click('promptAccepted', '#prompt', (dialog) => dialog.accept('answer!'))
    await click('promptAcceptedWithoutText', '#prompt', (dialog) => dialog.accept())
    await click('promptDismissed', '#prompt', (dialog) => dialog.dismiss())
    await click('bareAcceptedEmpty', '#prompt-bare', (dialog) => dialog.accept(''))
    await click('confirmAccepted', '#confirm', (dialog) => dialog.accept())
    const [closed] = await Promise.all([
      page.waitForEvent('dialogclosed'),
      click('confirmDismissed', '#confirm', (dialog) => dialog.dismiss()),
    ])
    return { results, dialogs, dismissedClosed: [closed.type(), closed.message()] }`,
    // accept() without text answers '', not the prompt's default.
    expected: {
      results: {
        promptAccepted: '"answer!"',
        promptAcceptedWithoutText: '""',
        promptDismissed: 'null',
        bareAcceptedEmpty: '""',
        confirmAccepted: 'true',
        confirmDismissed: 'false',
      },
      dialogs: [
        ['prompt', 'question?', 'yes.'],
        ['prompt', 'question?', 'yes.'],
        ['prompt', 'question?', 'yes.'],
        ['prompt', 'question?', ''],
        ['confirm', 'boolean?', ''],
        ['confirm', 'boolean?', ''],
      ],
      dismissedClosed: ['confirm', 'boolean?'],
    },
    probe: 'window.__answers',
    effect: ['"answer!"', '""', 'null', '""', 'true', 'false'],
  },
  {
    // page-dialog.spec.ts "DI-013 should auto-dismiss the alert without listeners", "DI-012
    // should auto-dismiss the prompt without listeners", "DI-004 should fire dialogclosed for
    // auto-dismissed dialogs".
    name: 'with no listener the click resolves and the dialog is dismissed',
    path: '/dialogs.html',
    source: `const results = {}
    for (const selector of ['#alert', '#confirm', '#prompt']) {
      await page.click(selector)
      results[selector] = await page.textContent('#result')
    }
    const [closed] = await Promise.all([page.waitForEvent('dialogclosed'), page.click('#alert')])
    const accept = (dialog) => void dialog.accept()
    page.on('dialog', accept)
    await page.click('#confirm')
    const withListener = await page.textContent('#result')
    page.off('dialog', accept)
    await page.click('#confirm')
    return {
      results,
      closed: [closed.type(), closed.message(), closed.page() === page],
      withListener,
      afterOff: await page.textContent('#result'),
    }`,
    expected: {
      results: {
        '#alert': '"undefined"',
        '#confirm': 'false',
        '#prompt': 'null',
      },
      closed: ['alert', 'yo', true],
      withListener: 'true',
      afterOff: 'false',
    },
    probe: 'window.__answers',
    effect: ['"undefined"', 'false', 'null', '"undefined"', 'true', 'false'],
  },
]

test('dialogs are answered the same through Neo and Playwright', {
  skip,
  timeout: 300_000,
}, async (t) => {
  await compareEvents(t, {
    pages: { '/dialogs.html': fixture('dialogs.html') },
    cases,
  })
})
