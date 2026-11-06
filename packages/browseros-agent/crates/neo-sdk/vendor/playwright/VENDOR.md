# Vendored Playwright

Copied unchanged from https://github.com/microsoft/playwright at commit `26137face0500cacddf90c767dc1364da7262559`,
read from that commit's objects with `git show 26137face:<path>`. Do not edit anything here:
`bun run test` (`tests/unit/vendor.test.ts`) fails on any edit, addition or removal.

The files are Playwright's selector builders and the escapers they use, at their paths under
Playwright's `packages/`, plus its LICENSE and NOTICE. `src/locator.ts` and `src/frame.ts` build
every locator's selector with them, which is what keeps Neo's selectors byte-identical to
Playwright's (`tests/goldens/selector-goldens.json`).

| File | sha256 |
| --- | --- |
| `isomorphic/locatorUtils.ts` | `95355975e13ca8185df9b9ad97ed2b982981ca50d455b7ed974bcf08fba2f506` |
| `isomorphic/stringUtils.ts` | `4ef81deedff640b6b12d55ae496252fb8b5825fc0925738096bd164c787e75f9` |
| `LICENSE` | `45873d00a0dd243596deb4aa23b2493b3d1f0671921bf2538ea431d7380220eb` |
| `NOTICE` | `6d602191187b35b9b01d2cffa01c8469c2c8d9de8a96f1bf868e0f264f51c81d` |
