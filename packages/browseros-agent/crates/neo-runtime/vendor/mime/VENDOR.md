# Vendored mime

The type tables of the `mime` package, version 4.1.0 (https://github.com/broofa/mime), copied
unchanged from the `node_modules/mime` of Playwright's checkout at commit
`26137face0500cacddf90c767dc1364da7262559` (its `package.json` pins `"mime": "4.1.0"`), with the
package's MIT LICENSE. Playwright's server types upload and drop files with this package
(`server/fileUploadUtils.ts`, `server/dom.ts`: `mime.getType`); neo-runtime's `mime.rs` reads
these two tables in the order the package's default export defines them. Do not edit.

| File | sha256 |
| --- | --- |
| `LICENSE` | `68e28d2ab9d180b8fc3d93fac0cc3e5275e4d19a23ab3dcf5458cf4e31e2233b` |
| `types/standard.ts` | `6cd8a657c4aa48aa685958694fea19693dd0865a362d83edcb01f13558eb95d4` |
| `types/other.ts` | `8b7358d606c6bc095de2a6b62da37853fac278269a34602ef3d9d2bbd6813bb9` |
