# Vendored Playwright

Copied unchanged from https://github.com/microsoft/playwright at commit `26137face0500cacddf90c767dc1364da7262559`,
read from that commit's objects with `git show 26137face:<path>`. Do not edit anything here:
`bun run test` (`tests/unit/vendor.test.ts`) fails on any edit, addition or removal.

The files are Playwright's selector builders and the escapers they use, and its locator
generators with the selector and CSS parsers they import, at their paths under Playwright's
`packages/`, plus its LICENSE and NOTICE. `src/locator.ts` and `src/frame.ts` build every
locator's selector with them, which is what keeps Neo's selectors byte-identical to Playwright's
(`tests/goldens/selector-goldens.json`); `Locator.toString` and `Locator.description` read a
selector back with the generators, as Playwright's client does.

| File | sha256 |
| --- | --- |
| `isomorphic/cssParser.ts` | `8dd56c492bac213b36bee46761749619cf14d263a5cc44280e7fc6e3ff23ba4b` |
| `isomorphic/cssTokenizer.ts` | `beffbe943b63c10af3cfa325e163e7593f91f0824e18a5f7f77062b119e386bc` |
| `isomorphic/locatorGenerators.ts` | `0d1e1d20c4ce2154cb1980574b4b53c0059c7936c6157d2c6165c2fbcef86306` |
| `isomorphic/locatorUtils.ts` | `95355975e13ca8185df9b9ad97ed2b982981ca50d455b7ed974bcf08fba2f506` |
| `isomorphic/selectorParser.ts` | `f8f7f8b97d0c4d40c54cf2101f273490171a05a569d292196e56e02d11a0acc1` |
| `isomorphic/stringUtils.ts` | `4ef81deedff640b6b12d55ae496252fb8b5825fc0925738096bd164c787e75f9` |
| `LICENSE` | `45873d00a0dd243596deb4aa23b2493b3d1f0671921bf2538ea431d7380220eb` |
| `NOTICE` | `6d602191187b35b9b01d2cffa01c8469c2c8d9de8a96f1bf868e0f264f51c81d` |
