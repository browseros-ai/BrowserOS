# Browser tests

For standalone CDP tests, the Playwright corpus, or the claw-mcp contract suite, read
[Live browser tests](../../CLAUDE.md#live-browser-tests) before choosing a launcher.

# Heavy jobs

Run these on the remote pool when you can; read
[Remote heavy jobs](../../CLAUDE.md#remote-heavy-jobs) first.

- Every SDK change: the gate, `check.sh` (`bun run remote gate`).
- The image (`neo-server/docker/`), or behaviour one of its checks covers (listed at the top of
  `neo-server/docker/accept.sh`, whichever crate holds it): also `bun run remote image`.
- Browser operations or the corpus (`neo-playwright`, `neo-playwright-js`,
  `neo-playwright-tests`): also `bun run remote spec --ci`.
