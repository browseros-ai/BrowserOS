// Bun/Node subpath compatibility shim.
// Some runtimes (notably Bun) may not honor `package.json#exports` reliably for TS source.
// This file makes `@browseros/shared/constants/exit-codes` resolvable via normal file lookup.
export * from '../src/constants/exit-codes'
