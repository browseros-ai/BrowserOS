globalThis.__ultrafast = (() => {
  const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
  // Page methods that change something. In verify and triage modes they prove their target
  // exists and record what they would have done, but never act.
  const WRITES = new Set([
    'click', 'fill', 'selectOption', 'check', 'uncheck', 'press', 'type', 'insertText',
    'upload', 'drag', 'dialogAccept', 'clickAt', 'typeAt', 'dragAt', 'download',
  ]);

  function HandBack(kind, data) {
    this.kind = kind;
    this.data = data || {};
  }

  function build(steps) {
    return steps.map((step) => {
      try {
        return { ...step, fn: new AsyncFunction('ctx', `return (${step.src})(ctx);`) };
      } catch (error) {
        throw new HandBack('syntax', { step: step.n, message: String(error) });
      }
    });
  }

  function tracked(browser, st) {
    const remember = (page) => {
      st.lastPage = page;
      if (!st.opened.includes(page)) st.opened.push(page);
      return page;
    };
    return {
      ...browser,
      open: async (...args) => remember(await browser.open(...args)),
      page: (id) => {
        const page = browser.page(id);
        st.lastPage = page;
        return page;
      },
    };
  }

  function dryWrites(page, st) {
    return new Proxy(page, {
      get(target, prop) {
        if (typeof prop === 'string' && WRITES.has(prop)) {
          return async (ref) => {
            if (typeof ref === 'string') await target.resolveRef(ref);
            st.dry.push({ method: prop, ref: typeof ref === 'string' ? ref : null });
            return { dryRun: true };
          };
        }
        return target[prop];
      },
    });
  }

  function message(error) {
    if (error && typeof error === 'object' && 'message' in error) return String(error.message);
    return String(error);
  }

  // A step that hangs must still hand back before the run tool's hard cap kills the script.
  function withinDeadline(promise, deadline, n) {
    const left = deadline - Date.now();
    if (left <= 0) return Promise.reject(new HandBack('time_cap', { step: n }));
    return Promise.race([
      promise,
      sleep(left).then(() => { throw new HandBack('time_cap', { step: n }); }),
    ]);
  }

  async function closeOpened(st) {
    for (const page of st.opened) {
      try { await page.close(); } catch (_) {}
    }
  }

  async function handOver(failure, st, o) {
    let page = null;
    let view = null;
    if (st.lastPage) {
      try { page = await st.lastPage.info(); } catch (_) {}
      try {
        const snapshot = await st.lastPage.snapshot({ mode: 'interactive' });
        view = String(snapshot.text || '').slice(0, o.viewBytes);
      } catch (_) {}
    }
    return {
      status: 'needs_agent',
      kind: failure.kind,
      step: failure.data.step ?? null,
      data: failure.data,
      done: st.done,
      committed: st.committed,
      page,
      view,
      dry: st.dry,
    };
  }

  async function run(browser, o) {
    const st = { done: [], committed: false, lastPage: null, opened: [], dry: [] };
    const started = Date.now();
    const deadline = started + o.deadlineMs;
    const live = o.mode === 'live';
    let steps;
    try {
      steps = build(o.steps);
    } catch (failure) {
      return { status: 'needs_agent', kind: failure.kind, step: failure.data.step, data: failure.data, done: [], committed: false, page: null, view: null, dry: [] };
    }
    if (o.mode === 'parse') return { status: 'parsed', steps: steps.length };

    const state = {};
    const handBack = (kind, data) => { throw new HandBack(String(kind), data || {}); };
    const firstOf = async (...strategies) => {
      for (const strategy of strategies) {
        try {
          const value = await strategy();
          if (value !== undefined && value !== null && value !== false) return value;
        } catch (error) {
          if (error instanceof HandBack) throw error;
        }
      }
      return null;
    };
    const ctx = { browser: tracked(browser, st), inputs: o.inputs, state, handBack, firstOf };

    let output;
    try {
      for (const step of steps) {
        if (o.stopAfter != null && step.n > o.stopAfter) {
          await closeOpened(st);
          return { status: 'reached', step: o.stopAfter, done: st.done, dry: st.dry };
        }
        if (Date.now() > deadline) throw new HandBack('time_cap', { step: step.n });
        const stepStarted = Date.now();
        if (step.commit && !live) {
          if (!state.page) {
            throw new HandBack('commit_outside_page', {
              step: step.n,
              message: 'the commit step needs the working page in state.page',
            });
          }
          const page = state.page;
          state.page = dryWrites(page, st);
          try {
            await withinDeadline(step.fn(ctx), deadline, step.n);
          } catch (error) {
            if (error instanceof HandBack) throw Object.assign(error, { data: { step: step.n, ...error.data } });
            throw new HandBack('exception', { step: step.n, message: message(error) });
          } finally {
            state.page = page;
          }
          if (st.dry.length === 0) {
            throw new HandBack('commit_outside_page', {
              step: step.n,
              message: 'the commit step did not act on state.page',
            });
          }
          st.done.push({ n: step.n, ms: Date.now() - stepStarted, dryRun: true });
          await closeOpened(st);
          return { status: 'verified_to_commit', done: st.done, dry: st.dry, durationMs: Date.now() - started };
        }
        if (step.commit) st.committed = 'in_progress';
        try {
          output = await withinDeadline(step.fn(ctx), deadline, step.n);
        } catch (error) {
          if (step.commit) st.committed = 'unknown';
          if (error instanceof HandBack) throw Object.assign(error, { data: { step: step.n, ...error.data } });
          throw new HandBack('exception', { step: step.n, message: message(error) });
        }
        if (step.commit) st.committed = true;
        st.done.push({ n: step.n, ms: Date.now() - stepStarted });
      }
      await closeOpened(st);
      return { status: 'done', output: output ?? null, done: st.done, durationMs: Date.now() - started };
    } catch (failure) {
      const handed = failure instanceof HandBack
        ? failure
        : new HandBack('exception', { message: message(failure) });
      const packet = await handOver(handed, st, o);
      if (!live) await closeOpened(st);
      packet.durationMs = Date.now() - started;
      return packet;
    }
  }

  return { run };
})();
