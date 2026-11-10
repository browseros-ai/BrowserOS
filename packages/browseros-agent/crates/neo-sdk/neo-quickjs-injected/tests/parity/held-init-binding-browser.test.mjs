import { test } from 'node:test'
import { skip } from './harness.mjs'
import { compareHeldInitBinding } from './held-init-binding.mjs'

test(
  'tests that a held init-script removal does not skip binding cleanup at browser.close',
  {
    skip,
    timeout: 45_000,
  },
  (t) => compareHeldInitBinding(t, 'context.browser().close()'),
)
