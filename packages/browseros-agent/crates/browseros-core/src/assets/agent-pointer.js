// biome-ignore-all lint: Injected through Runtime.evaluate; Rust fills the __BROWSEROS_POINTER_*__ placeholders.
// biome-ignore format: Keep the injected expression byte-oriented.
(async (cmd) => {
  // Placeholder until the page wrapper lands; it never draws.
  const loadCursor = () => {
    __BROWSEROS_POINTER_CURSOR__
  };
  void loadCursor;
  void cmd;
  return { status: 'skipped', reason: 'placeholder' };
})(__BROWSEROS_POINTER_CMD__)
