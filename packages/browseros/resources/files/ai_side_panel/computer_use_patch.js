(() => {
  const existing = globalThis.fetch;
  if (!existing || existing.__browserosComputerUsePatched) return;

  const originalFetch = existing.bind(globalThis);
  const computerUseModels = new Set(["gemini-2.5-computer-use-preview-10-2025"]);

  const normalizePayload = (payload) => {
    if (!payload || typeof payload !== "object") return null;
    if (!computerUseModels.has(payload.model)) return null;

    const tools = Array.isArray(payload.tools) ? payload.tools.slice() : [];
    const hasTool = tools.some((tool) => tool && typeof tool === "object" && "computer_use" in tool);

    if (!hasTool) {
      tools.push({ computer_use: { environment: "ENVIRONMENT_BROWSER" } });
      payload.tools = tools;
      return payload;
    }

    return payload;
  };

  const updateInit = (init) => {
    if (!init || typeof init !== "object") return null;
    if (!init.body) return null;

    if (typeof init.body === "string") {
      try {
        const payload = JSON.parse(init.body);
        const updated = normalizePayload(payload);
        if (updated) {
          return { ...init, body: JSON.stringify(updated) };
        }
      } catch {
        return null;
      }
    } else if (typeof init.body === "object") {
      const updated = normalizePayload(init.body);
      if (updated) {
        return { ...init, body: JSON.stringify(updated) };
      }
    }

    return null;
  };

  const wrappedFetch = (...args) => {
    const [input, init] = args;
    const nextInit = updateInit(init);

    if (nextInit) {
      return originalFetch(input, nextInit);
    }

    return originalFetch(...args);
  };

  wrappedFetch.__browserosComputerUsePatched = true;
  globalThis.fetch = wrappedFetch;
})();
