/**
 * The vendored Playwright v1.64.1 helpers the prelude is built on, imported from the crate's
 * unmodified copy (crates/neo-sdk/neo-playwright/third_party/playwright) in one place.
 *
 * These are the functions playwright-core's own client uses to build selector strings and to
 * print locators, so a locator built here is byte for byte the one real Playwright builds
 * (tests/selector-goldens.test.ts). The bundler tree-shakes each module to what is named here,
 * which keeps code that needs `URL` (urlMatch's `urlMatches`) out of the QuickJS bundle.
 */
export type {
  AriaNodeJSON,
  AriaSnapshotJSON,
} from '../../../neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshot'
export { renderAriaSnapshotAsYaml } from '../../../neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshotRenderer'
export {
  asLocator,
  asLocatorDescription,
  locatorCustomDescription,
} from '../../../neo-playwright/third_party/playwright/packages/isomorphic/locatorGenerators'
export {
  type ByRoleOptions,
  getByAltTextSelector,
  getByLabelSelector,
  getByPlaceholderSelector,
  getByRoleSelector,
  getByTestIdSelector,
  getByTextSelector,
  getByTitleSelector,
} from '../../../neo-playwright/third_party/playwright/packages/isomorphic/locatorUtils'
export {
  kAnyFrameSelector,
  type ParsedSelector,
  splitSelectorByFrame,
} from '../../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser'
export {
  escapeForTextSelector,
  escapeRegExp,
} from '../../../neo-playwright/third_party/playwright/packages/isomorphic/stringUtils'
export { globToRegexPattern } from '../../../neo-playwright/third_party/playwright/packages/isomorphic/urlMatch'
