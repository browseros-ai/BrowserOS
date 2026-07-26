import { defineConfig } from 'drizzle-kit'

export default defineConfig({
  dialect: 'sqlite',
  schema: './src/lib/db/schema/index.ts',
  out: './src/lib/db/migrations',
})
