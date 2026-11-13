# Browser tests

For standalone CDP tests, the Playwright corpus, or the claw-mcp contract suite, read
[Live browser tests](../../CLAUDE.md#live-browser-tests) before choosing a launcher.

# Heavy jobs

Run these on the remote pool when you can; read
[Remote heavy jobs](../../CLAUDE.md#remote-heavy-jobs) first.

- Every SDK change: the gate, `check.sh` (`bun run remote gate`).
- The server, its host API or its image (`neo-server`, `neo-http`, `neo-server/docker/`): also
  `bun run remote image`.
- Browser operations (`neo-playwright`, `neo-playwright-js`): also `bun run remote spec --ci`.
