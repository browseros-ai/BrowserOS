// The oracle's reader: what Playwright's own ARIA engine says about one frame's document.
//
// The recorder (examples/record_fixtures.rs, via tests/support/oracle_capture.rs) splices this
// source into neo-playwright's assets/injected.js right after the bundle's one
// `globalThis.__neoProbe ??= createProbe();` statement, inside its IIFE. There it sees the
// bundle's top-level bindings (roleUtils, ariaSnapshot, Neo's probe helpers) under their own
// names, unmodified: the oracle is the shipped engine, not a copy of it. The patched bundle is
// evaluated in a "neo-oracle" isolated world of its own, so its prototype patches (shadow-root
// getters, masking scopes) never reach the page or Neo's world.
//
// The reply is one JSON string, as the probe's replyText makes it: an ARIA tree nests two JSON
// levels per element, and the CDP client refuses a by-value reply past serde's 128 levels.
;(() => {
  // A renamed or tree-shaken binding must fail the recording by name, never produce an oracle
  // that silently reads something else. `typeof` of an undeclared name is 'undefined'.
  const bindings = {
    InjectedScript: typeof InjectedScript,
    createEngine: typeof createEngine,
    getAriaRole: typeof getAriaRole,
    getElementAccessibleName: typeof getElementAccessibleName,
    getElementAccessibleDescription: typeof getElementAccessibleDescription,
    getElementAccessibleErrorMessage: typeof getElementAccessibleErrorMessage,
    getAriaChecked: typeof getAriaChecked,
    getAriaPressed: typeof getAriaPressed,
    getAriaExpanded: typeof getAriaExpanded,
    getAriaLevel: typeof getAriaLevel,
    getAriaSelected: typeof getAriaSelected,
    getAriaDisabled: typeof getAriaDisabled,
    getAriaInvalid: typeof getAriaInvalid,
    kAriaCheckedRoles: typeof kAriaCheckedRoles,
    kAriaPressedRoles: typeof kAriaPressedRoles,
    kAriaExpandedRoles: typeof kAriaExpandedRoles,
    kAriaLevelRoles: typeof kAriaLevelRoles,
    kAriaSelectedRoles: typeof kAriaSelectedRoles,
    kAriaDisabledRoles: typeof kAriaDisabledRoles,
    kAriaInvalidRoles: typeof kAriaInvalidRoles,
    beginAriaCaches: typeof beginAriaCaches,
    endAriaCaches: typeof endAriaCaches,
    isElementHiddenForAria: typeof isElementHiddenForAria,
    isElementVisible: typeof isElementVisible,
    normalizeWhiteSpace: typeof normalizeWhiteSpace,
    withMaskedFieldValues: typeof withMaskedFieldValues,
    installShadowRoots: typeof installShadowRoots,
    openClosedRoots: typeof openClosedRoots,
    lastSnapshotRefs: typeof lastSnapshotRefs,
  }
  for (const [name, type] of Object.entries(bindings)) {
    if (type === 'undefined')
      throw new Error(`neo-oracle: injected.js no longer defines ${name}`)
  }

  // The states toAriaNode (ariaSnapshot.ts) writes, offered to the same roles, so a state is
  // present here exactly when Playwright's tree could print it.
  function statesOf(element, role) {
    const states = {}
    if (kAriaCheckedRoles.includes(role))
      states.checked = getAriaChecked(element)
    if (kAriaDisabledRoles.includes(role))
      states.disabled = getAriaDisabled(element)
    if (kAriaExpandedRoles.includes(role)) {
      const expanded = getAriaExpanded(element)
      if (expanded !== undefined) states.expanded = expanded
    }
    if (kAriaInvalidRoles.includes(role)) {
      const invalid = getAriaInvalid(element)
      states.invalid =
        invalid === 'false' ? false : invalid === 'true' ? true : invalid
    }
    if (kAriaLevelRoles.includes(role)) states.level = getAriaLevel(element)
    if (kAriaPressedRoles.includes(role))
      states.pressed = getAriaPressed(element)
    if (kAriaSelectedRoles.includes(role))
      states.selected = getAriaSelected(element)
    const owner = element.ownerDocument
    states.active = owner.activeElement === element && owner.hasFocus()
    return states
  }

  // Every element of this document in tree order, a host's shadow root before its light
  // children, as the engine's own walk enters them. `shadowRoot` is this world's getter, which
  // answers for the closed roots openClosedRoots was handed. `children` never yields a
  // template's content or another document, so neither is entered.
  function elementsInTreeOrder(root) {
    const ordered = []
    const pending = root ? [root] : []
    while (pending.length > 0) {
      const element = pending.pop()
      ordered.push(element)
      const shadow = element.shadowRoot
      const children = [...(shadow ? shadow.children : []), ...element.children]
      for (let i = children.length - 1; i >= 0; i--) pending.push(children[i])
    }
    return ordered
  }

  globalThis.__neoOracleRead = (seq, ...seeds) => {
    // Neo's marked snapshots open closed roots the same way, before the engine is built.
    installShadowRoots()
    openClosedRoots(seeds)
    const injected = createEngine(seq)

    // One index per distinct element, shared by element facts and refs. Rust reads the array
    // back by remote object and turns each index into a backendNodeId.
    const nodes = []
    const indexes = new Map()
    const nodeOf = (element) => {
      let index = indexes.get(element)
      if (index === undefined) {
        index = nodes.length
        nodes.push(element)
        indexes.set(element, index)
      }
      return index
    }

    // Neo's snapshot scope (failure-output.ts `tree`, declared fields masked): a private box's
    // value reads as bullets here exactly as Neo prints it today.
    const read = () => {
      const root = document.body ?? document.documentElement
      const ai = injected.ariaSnapshotJSON(root, { mode: 'ai' }).json
      // The next ariaSnapshotJSON call replaces the engine's ref table, so read it now. It keeps
      // refs the distiller removed from the printed tree: they still resolve.
      const refs = []
      for (const [ref, info] of lastSnapshotRefs(injected))
        refs.push({ ref, node: nodeOf(info.element) })
      const defaultTree = injected.ariaSnapshotJSON(root, {
        mode: 'default',
      }).json

      const elements = []
      // Each option a select lists, by the select's own list: the list Chromium copies into APC's
      // select_options, whose entries are a collapsed select's option slots in the tree. Read from
      // the select's side, not the option's (Neo's probe goes from the option, option-slot.ts
      // slotOf), so the join the tests make is checked against separate evidence. A select comes
      // before its options in tree order, so the map is full by the time they are read.
      const listed = new Map()
      beginAriaCaches()
      try {
        for (const element of elementsInTreeOrder(document.documentElement)) {
          if (element instanceof HTMLSelectElement) {
            const { options } = element
            for (let index = 0; index < options.length; index++)
              listed.set(options[index], {
                select: nodeOf(element),
                index,
                count: options.length,
              })
          }
          const role = getAriaRole(element)
          if (role === null) continue
          elements.push({
            node: nodeOf(element),
            role,
            name: normalizeWhiteSpace(
              getElementAccessibleName(element, false).text,
            ),
            description: normalizeWhiteSpace(
              getElementAccessibleDescription(element, false).text,
            ),
            errorMessage: getElementAccessibleErrorMessage(element),
            hidden: isElementHiddenForAria(element),
            visible: isElementVisible(element),
            states: statesOf(element, role),
            option: listed.get(element),
          })
        }
      } finally {
        endAriaCaches()
      }
      return { elements, refs, aria: { ai, default: defaultTree } }
    }
    const result = withMaskedFieldValues(
      { root: document.documentElement, discoverMovedFields: false },
      read,
      { declared: true },
    )
    globalThis.__neoOracleNodes = nodes
    return JSON.stringify(result)
  }
})()
