// biome-ignore-all lint: Injected into an isolated world; kept byte-oriented like core's other page assets.
// The sealed world (sdk-privacy L1). Runs once per document in Neo's own isolated world, named
// SEALED_WORLD in seal.rs, before any other Neo code there.
//
// An isolated world shares the page's DOM but has its own JavaScript prototypes. Redefining the
// getters a value can be read through therefore changes what Neo's code in this world reads and
// leaves the page's own scripts untouched: every read of a password, card or one-time-code
// field made from here returns "••••" (a card number keeps its last four). The native getters
// survive only inside this closure, for jobs that never hand a value to the agent:
//   kind(el)                 which fields are sensitive (also traces' sensitive_target)
//   valueEquals(el, s)       expect(...).toHaveValue compares the real value, reports the mask
//   attributeEquals(el, n, s) expect(...).toHaveAttribute compares the markup's raw attribute,
//                            which an edit of the field leaves as it was
//   rawLength(el)            selectText needs the real length
//   sensitiveValues()        the sweep: Rust masks these exact values in a main-world result
//   watch(root)              journal and sweep a closed shadow root Rust found, while it lives;
//                            or sweep an element or fragment out of the document, and follow its
//                            fields' types, while it lives
//   detachedTops()           the tops of the watched trees out of the document, for Rust's scan
//   takeHost()               a shadow host the journal reported, for Rust to scan
//
// The journal reports each value entered into a sensitive field through a CDP binding that was
// added with executionContextName = this world, so only this world has it. Rust registers the
// values with the secret set (sdk-traces' Privacy). It also reports HIDDEN_ROOT_REPORT (seal.rs)
// when typing reaches a listener here from inside a closed root no one watches yet.
//
// Prototypes are per frame. Code here must never reach into another frame's document (it would
// read through that frame's getters, which may not be sealed yet), so contentDocument and
// contentWindow return null here; Rust hops frames itself and seals each one.
//
// seal.rs passes rules.json as the argument at the bottom; field_rules.rs evaluates the same
// table over snapshot data, and the live parity test keeps the two evaluators in agreement.
// biome-ignore format: Keep the injected script byte-oriented, like core's other page assets.
(function (rules) {
  if (globalThis.__clawSeal) return globalThis.__clawSeal.version;

  var FIELD_MASK = "••••";
  var desc = function (proto, name) { return Object.getOwnPropertyDescriptor(proto, name); };
  var raw = {
    inputValue: desc(HTMLInputElement.prototype, "value"),
    inputDefault: desc(HTMLInputElement.prototype, "defaultValue"),
    inputNumber: desc(HTMLInputElement.prototype, "valueAsNumber"),
    textareaValue: desc(HTMLTextAreaElement.prototype, "value"),
    textareaDefault: desc(HTMLTextAreaElement.prototype, "defaultValue"),
    selectValue: desc(HTMLSelectElement.prototype, "value"),
    attrValue: desc(Attr.prototype, "value"),
    nodeValue: desc(Node.prototype, "nodeValue"),
    textContent: desc(Node.prototype, "textContent"),
    characterData: desc(CharacterData.prototype, "data"),
    wholeText: desc(Text.prototype, "wholeText"),
    innerText: desc(HTMLElement.prototype, "innerText"),
    outerHTML: desc(Element.prototype, "outerHTML"),
    innerHTML: desc(Element.prototype, "innerHTML"),
    getAttribute: Element.prototype.getAttribute,
    getAttributeNS: Element.prototype.getAttributeNS,
    elementQueryAll: Element.prototype.querySelectorAll,
    fragmentQueryAll: DocumentFragment.prototype.querySelectorAll,
    documentQueryAll: Document.prototype.querySelectorAll,
    closest: Element.prototype.closest,
    selectionToString: Selection.prototype.toString,
    addEventListener: EventTarget.prototype.addEventListener,
    composedPath: Event.prototype.composedPath,
    stringify: JSON.stringify,
  };
  var attr = function (el, name) { return raw.getAttribute.call(el, name); };
  var text = function (node) { return node ? String(raw.textContent.get.call(node) || "") : ""; };
  var lower = function (value) { return String(value || "").trim().toLowerCase(); };
  var isInput = function (el) { return el instanceof HTMLInputElement; };
  var isField = function (el) {
    return el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement;
  };
  var rawValue = function (el) {
    if (el instanceof HTMLInputElement) return raw.inputValue.get.call(el);
    if (el instanceof HTMLTextAreaElement) return raw.textareaValue.get.call(el);
    if (el instanceof HTMLSelectElement) return raw.selectValue.get.call(el);
    return "";
  };
  var queryAll = function (root, selector) {
    if (root instanceof Document) return raw.documentQueryAll.call(root, selector);
    if (root instanceof Element) return raw.elementQueryAll.call(root, selector);
    return raw.fragmentQueryAll.call(root, selector);
  };

  // ---- classification: the same table and order as field_rules.rs ----
  var codeSlotContext = new RegExp(rules.codeSlot.context, "i");
  var words = rules.words.map(function (rule) { return { kind: rule.kind, pattern: new RegExp(rule.pattern, "i") }; });
  var wasPassword = new WeakSet();
  var includes = function (list, value) { return list.indexOf(value) !== -1; };

  // Rust classifies with Chromium's accessible name. This stand-in reads a label's text plus the
  // alt, aria-label and title of elements inside it, so <label><img alt="Password"></label>
  // names the field here too. It may read more words than the accessible name, never fewer of
  // these, so a disagreement masks in the sealed world rather than leaking from it.
  function labelText(node) {
    var parts = [text(node)];
    var inner = raw.elementQueryAll.call(node, "[alt],[aria-label],[title]");
    for (var i = 0; i < inner.length; i++) parts.push(attr(inner[i], "alt"), attr(inner[i], "aria-label"), attr(inner[i], "title"));
    return parts.filter(Boolean).join(" ");
  }

  // The first element of root's tree with this id. The top of a tree the page removed may be
  // an element, which has no getElementById, and the document's would search another tree.
  function byId(root, id) {
    if (root.getElementById) return root.getElementById(id);
    if (attr(root, "id") === id) return root;
    var found = raw.elementQueryAll.call(root, "[id]");
    for (var i = 0; i < found.length; i++) if (attr(found[i], "id") === id) return found[i];
    return null;
  }

  // Chromium answers el.labels from the document's tree even for a field the page removed with
  // its labels, so it finds none of them (measured); a shadow root's tree is its own wherever
  // its host is. Out of the document the field's own tree is searched instead: every label
  // around it, and every label whose for names it. That may be more labels than HTML's, never
  // fewer, so the field keeps the words it had in the document.
  function labelsOf(el) {
    var root = el.getRootNode();
    if (root instanceof Document || root instanceof ShadowRoot) return el.labels || [];
    var found = [];
    for (var up = raw.closest.call(el, "label"); up; up = up.parentElement && raw.closest.call(up.parentElement, "label")) found.push(up);
    var id = attr(el, "id");
    var named = id ? queryAll(root, "label") : [];
    for (var i = 0; i < named.length; i++) if (attr(named[i], "for") === id && found.indexOf(named[i]) === -1) found.push(named[i]);
    return found;
  }

  function label(el) {
    var parts = [];
    var labels = labelsOf(el);
    for (var i = 0; i < labels.length; i++) parts.push(labelText(labels[i]));
    var labelledBy = attr(el, "aria-labelledby");
    if (labelledBy) {
      var root = el.getRootNode();
      labelledBy.split(/\s+/).forEach(function (id) {
        var node = byId(root, id);
        if (node) parts.push(labelText(node));
      });
    }
    return parts.join(" ");
  }

  function wordsOf(el) {
    return [attr(el, "name"), attr(el, "id"), attr(el, "aria-label"), attr(el, "placeholder"), attr(el, "title"), label(el)]
      .filter(Boolean).join(" ");
  }

  /** null when the field holds no secret, else "password" | "one_time_code" | "card". */
  function kind(el) {
    if (!isField(el)) return null;
    var type = isInput(el) ? lower(attr(el, "type")) : "";
    if (isInput(el) && includes(rules.excludedTypes, type)) return null;
    if (isInput(el)) observeTypes(el.getRootNode());
    if (isInput(el) && includes(rules.passwordTypes, type)) wasPassword.add(el);
    if (isInput(el) && wasPassword.has(el)) return "password";
    var tokens = lower(attr(el, "autocomplete")).split(/\s+/).filter(Boolean);
    if (tokens.some(function (t) { return includes(rules.passwordTokens, t); })) return "password";
    if (tokens.some(function (t) { return includes(rules.oneTimeCodeTokens, t); })) return "one_time_code";
    if (tokens.some(function (t) { return rules.cardTokenPrefixes.some(function (p) { return t.indexOf(p) === 0; }); })) return "card";
    var fieldWords = wordsOf(el);
    var slot = rules.codeSlot;
    var maxLength = attr(el, "maxlength");
    var mode = lower(attr(el, "inputmode")) || type;
    if (maxLength !== null && Number(maxLength) === slot.maxLength && includes(slot.inputModes, mode)) {
      var form = raw.closest.call(el, "form");
      if (form && (codeSlotContext.test(text(form).slice(0, rules.formTextLimit)) || codeSlotContext.test(fieldWords))) return "one_time_code";
    }
    for (var i = 0; i < words.length; i++) if (words[i].pattern.test(fieldWords)) return words[i].kind;
    return null;
  }

  function luhn(digits) {
    var sum = 0;
    for (var i = 0; i < digits.length; i++) {
      var n = Number(digits[digits.length - 1 - i]);
      if (i % 2 === 1) { n *= 2; if (n > 9) n -= 9; }
      sum += n;
    }
    return sum % 10 === 0;
  }

  /** mask_value.rs, in JavaScript. */
  function mask(k, value) {
    if (value === "" || value == null) return value;
    var v = String(value);
    if (k === "card" && /^[\d -]+$/.test(v)) {
      var digits = v.replace(/[^\d]/g, "");
      if (digits.length >= 13 && digits.length <= 19 && luhn(digits)) return FIELD_MASK + " " + digits.slice(-4);
    }
    return FIELD_MASK;
  }

  var masked = function (el, value) {
    if (value === "" || value == null) return value;
    var k = kind(el);
    return k ? mask(k, value) : value;
  };

  // ---- shadow every getter a value can be read through ----
  var shadowGet = function (proto, name, original, wrap) {
    if (!original || !original.get) return;
    Object.defineProperty(proto, name, {
      configurable: original.configurable,
      enumerable: original.enumerable,
      set: original.set,
      get: function () { return wrap(this, original.get.call(this)); },
    });
  };
  shadowGet(HTMLInputElement.prototype, "value", raw.inputValue, masked);
  shadowGet(HTMLInputElement.prototype, "defaultValue", raw.inputDefault, masked);
  shadowGet(HTMLInputElement.prototype, "valueAsNumber", raw.inputNumber, function (el, v) { return kind(el) ? NaN : v; });
  shadowGet(HTMLTextAreaElement.prototype, "value", raw.textareaValue, masked);
  shadowGet(HTMLTextAreaElement.prototype, "defaultValue", raw.textareaDefault, masked);
  shadowGet(HTMLSelectElement.prototype, "value", raw.selectValue, masked);
  var valueAttrOfSensitive = function (a) {
    return a && lower(a.name) === "value" && a.ownerElement && kind(a.ownerElement);
  };
  shadowGet(Attr.prototype, "value", raw.attrValue, function (a, v) { return valueAttrOfSensitive(a) ? masked(a.ownerElement, v) : v; });
  // A <textarea>'s child text is its initial value (what a page prefilled), so it is masked
  // like the value: read on its own, through its text node, or inside an ancestor's text.
  var inSensitiveTextarea = function (n) {
    return n instanceof Text && n.parentNode instanceof HTMLTextAreaElement && kind(n.parentNode);
  };
  var scrubText = function (root, v) {
    if (!v || !(root instanceof Element || root instanceof DocumentFragment)) return v;
    var areas = root instanceof Element ? raw.elementQueryAll.call(root, "textarea") : raw.fragmentQueryAll.call(root, "textarea");
    for (var i = 0; i < areas.length; i++) {
      var t = text(areas[i]);
      if (t && kind(areas[i])) v = v.split(t).join(mask(kind(areas[i]), t));
    }
    return v;
  };
  var nodeText = function (n, v) {
    if (n instanceof Attr) return valueAttrOfSensitive(n) ? masked(n.ownerElement, v) : v;
    if (n instanceof HTMLTextAreaElement) return masked(n, v);
    if (inSensitiveTextarea(n)) return masked(n.parentNode, v);
    return scrubText(n, v);
  };
  shadowGet(Node.prototype, "nodeValue", raw.nodeValue, nodeText);
  shadowGet(Node.prototype, "textContent", raw.textContent, nodeText);
  shadowGet(CharacterData.prototype, "data", raw.characterData, nodeText);
  shadowGet(Text.prototype, "wholeText", raw.wholeText, nodeText);
  shadowGet(HTMLElement.prototype, "innerText", raw.innerText, function (el, v) {
    return el instanceof HTMLTextAreaElement ? masked(el, v) : v;
  });
  Element.prototype.getAttribute = function (name) {
    var v = raw.getAttribute.call(this, name);
    return v && lower(name) === "value" ? masked(this, v) : v;
  };
  Element.prototype.getAttributeNS = function (ns, name) {
    var v = raw.getAttributeNS.call(this, ns, name);
    return v && !ns && lower(name) === "value" ? masked(this, v) : v;
  };
  // Markup: serialize, then replace each sensitive field's value="…" with its mask.
  var escapeAttr = function (s) {
    return s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/ /g, "&nbsp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  };
  var escapeText = function (s) {
    return s.replace(/&/g, "&amp;").replace(/\u00a0/g, "&nbsp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  };
  var scrubMarkup = function (root, html, includeRoot) {
    var fields = Array.prototype.slice.call(raw.elementQueryAll.call(root, "input,textarea,select"));
    if (includeRoot) fields.unshift(root);
    for (var i = 0; i < fields.length; i++) {
      if (fields[i] instanceof HTMLTextAreaElement) {
        var t = text(fields[i]);
        if (t && kind(fields[i])) {
          var shown = mask(kind(fields[i]), t);
          html = html.split(">" + escapeText(t) + "</textarea>").join(">" + shown + "</textarea>");
        }
      }
      var v = raw.getAttribute.call(fields[i], "value");
      if (!v || !kind(fields[i])) continue;
      var m = masked(fields[i], v);
      [escapeAttr(v), v.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/ /g, "&nbsp;")].forEach(function (form) {
        html = html.split('value="' + form + '"').join('value="' + m + '"');
      });
    }
    return html;
  };
  shadowGet(Element.prototype, "outerHTML", raw.outerHTML, function (el, html) { return scrubMarkup(el, html, true); });
  shadowGet(Element.prototype, "innerHTML", raw.innerHTML, function (el, html) {
    // A textarea's own innerHTML is its escaped text.
    if (el instanceof HTMLTextAreaElement && kind(el) && html) return mask(kind(el), text(el));
    return scrubMarkup(el, html, false);
  });
  // The focused control can sit inside a shadow root, where document.activeElement is only its
  // host. Follow open roots, and the closed ones Rust handed to watch().
  var deepActiveElement = function () {
    var active = document.activeElement;
    for (var depth = 0; depth < 32 && active; depth++) {
      var root = active.shadowRoot;
      if (!root) root = watchedHosts.get(active) || null;
      if (!root || !root.activeElement) break;
      active = root.activeElement;
    }
    return active;
  };
  Selection.prototype.toString = function () {
    var active = deepActiveElement();
    return active && kind(active) ? "" : raw.selectionToString.call(this);
  };
  // Neo's code here never needs these; removing them keeps a forgotten read from working.
  delete globalThis.FormData;
  delete globalThis.XMLSerializer;
  [HTMLIFrameElement.prototype, HTMLFrameElement.prototype, HTMLObjectElement.prototype, HTMLEmbedElement.prototype].forEach(function (proto) {
    ["contentDocument", "contentWindow"].forEach(function (name) {
      var original = desc(proto, name);
      if (original) Object.defineProperty(proto, name, { configurable: original.configurable, enumerable: original.enumerable, get: function () { return null; } });
    });
    if (typeof proto.getSVGDocument === "function") proto.getSVGDocument = function () { return null; };
  });

  // ---- show-password toggles: once a password, always a password in this document ----
  // An input is remembered as a password from the first time the seal sees it as one: in a root
  // it starts to observe, when it classifies it (kind, which every sweep runs on every field with
  // a value), or in a type change the watcher records. The page's toggle reaches the watcher
  // only inside a root it observes, and a field the page removes leaves the document's
  // observation: Chromium records no later toggle of it (measured). So the history must be
  // taken before the field leaves, by kind, or the tree it leaves in must be observed itself.
  var notePassword = function (el) {
    if (isInput(el) && includes(rules.passwordTypes, lower(attr(el, "type")))) wasPassword.add(el);
  };
  var notePasswords = function (root) {
    notePassword(root);
    var found = queryAll(root, "input");
    for (var i = 0; i < found.length; i++) notePassword(found[i]);
  };
  var typeWatcher = new MutationObserver(function (records) {
    for (var i = 0; i < records.length; i++) {
      var r = records[i];
      if (lower(r.oldValue) === "password" || lower(attr(r.target, "type")) === "password") wasPassword.add(r.target);
    }
  });
  // A MutationObserver on the document does not see changes inside shadow roots, nor in a tree
  // the page removed from it, so each root is observed on its own: at install for the roots
  // found then, and from a field's first classification (kind), a watch or a sweep for roots
  // that appear later, the top of a tree out of the document among them (the field itself when
  // the page removed just it). An observer holds the nodes it observes weakly, so a tree the page
  // drops still dies (measured).
  var observedRoots = new WeakSet();
  var observeTypes = function (root) {
    if (!root || observedRoots.has(root)) return;
    if (!(root instanceof Document || root instanceof DocumentFragment || root instanceof Element)) return;
    observedRoots.add(root);
    notePasswords(root);
    typeWatcher.observe(root, { subtree: true, attributes: true, attributeFilter: ["type"], attributeOldValue: true });
  };

  // ---- the journal ----
  var journal = typeof globalThis.__neoJournal === "function" ? globalThis.__neoJournal : null;
  delete globalThis.__neoJournal;
  var lastSent = new WeakMap();
  // The fields a trusted keydown, paste or drop reached since their last trusted entry (record).
  var granted = new WeakSet();
  // The closed roots Rust handed to watch(), held weakly: a component the page removed and
  // dropped dies with its root and is swept no more, and the values swept from it before stay
  // with the host's mask. A root the page keeps and attaches again is still watched, its
  // listeners still on it. By host, since a host's shadow root never changes.
  var watchedHosts = new WeakMap();
  var watchedRoots = [];
  // A composed event from inside a closed root reaches a listener outside it retargeted to the
  // root's host, whose shadowRoot is null from here. So typing into a closed root that no one
  // watches yet arrives as an input event at a host that is no field: the seal holds the host
  // and reports it at once, and Rust scans it, watches its root and sweeps it, even when the page
  // has removed the host by then. Each element is reported once (a contenteditable one reports
  // the same way, and Rust finds no root in it), and at most MAX_PENDING_HOSTS wait for Rust.
  var reportedHosts = new WeakSet();
  var pendingHosts = [];
  var MAX_PENDING_HOSTS = 16;
  var reportHost = function (event, el) {
    if (!journal || !event.composed || !(el instanceof Element) || isField(el)) return;
    if (el.shadowRoot || watchedHosts.has(el) || reportedHosts.has(el)) return;
    reportedHosts.add(el);
    if (pendingHosts.length >= MAX_PENDING_HOSTS) pendingHosts.shift();
    pendingHosts.push(el);
    journal('{"hiddenRoot":true}');
  };
  // One event grants once. A key reaches every root the seal listens on along its path (the
  // document, then each watched shadow root a slotted field sits in), and a page listener between
  // two of them could spend the grant and have the next root grant it again.
  var granting = new WeakSet();
  var grant = function (event) {
    var el = raw.composedPath.call(event)[0];
    if (!event.isTrusted || granting.has(event) || !isField(el)) return;
    granting.add(event);
    granted.add(el);
  };
  // An entry is trusted when the browser dispatched its event and a trusted keydown, paste or
  // drop reached the field since its last trusted entry. A trusted entry spends that grant, so
  // each of the owner's keys, pastes and drops makes at most one entry trusted. An untrusted
  // entry spends none: an input a page dispatches from its own keydown listener would otherwise
  // take the grant from the owner's keystroke. A page's script can dispatch none of the three
  // trusted, and its execCommand edits fire trusted input events but none of them, so those
  // report false unless they run inside the owner's key and spend its grant. Takeover's journal
  // registers an untrusted entry as a page value (JournalEntry::parse_with_trust in seal.rs).
  var record = function (event) {
    var path = raw.composedPath.call(event);
    var el = path && path[0];
    var k = kind(el);
    if (!k) return reportHost(event, el);
    if (!journal) return;
    var value = rawValue(el);
    var trusted = event.isTrusted && granted.has(el);
    // The same value again is sent only when it is trusted and the last was not: a value the
    // page reported first, that the owner then pastes or types over, becomes the owner's.
    var sent = lastSent.get(el);
    if (!value || (sent && sent.value === value && (sent.trusted || !trusted))) return;
    lastSent.set(el, { value: value, trusted: trusted });
    if (trusted) granted.delete(el);
    journal(raw.stringify({ kind: k, value: value, trusted: trusted }));
  };
  // The elements and fragments Rust handed to watch(): the nodes a main-world script receives,
  // which may be out of the document (a handle's node the page removed, or one it never
  // attached), where the document's sweep cannot see them. Held weakly, as the closed roots are.
  // Whether one is out of the document is read at every sweep, never at the watch: a node the
  // page removes later is swept from then on, and one it puts back is the document's again. A
  // tree already out at the watch has its types observed from then on, so a password the page
  // toggles before the next sweep stays one.
  var watchedTrees = [];
  var heldTrees = new WeakSet();
  // A repeat watch adds the listeners again: adding one that is already there changes nothing,
  // and document.open() (sdk-playwright's setContent) removes every listener of the document
  // while this world and its seal live on.
  var watch = function (root) {
    if (!root) return;
    if ((root instanceof Element || root instanceof DocumentFragment) && !(root instanceof ShadowRoot)) {
      if (!heldTrees.has(root)) { heldTrees.add(root); watchedTrees.push(new WeakRef(root)); }
      var top = root.getRootNode({ composed: true });
      if (top !== document) observeTypes(top);
      return;
    }
    raw.addEventListener.call(root, "input", record, true);
    raw.addEventListener.call(root, "change", record, true);
    raw.addEventListener.call(root, "keydown", grant, true);
    raw.addEventListener.call(root, "paste", grant, true);
    raw.addEventListener.call(root, "drop", grant, true);
    observeTypes(root);
    if (!(root instanceof ShadowRoot) || watchedHosts.has(root.host)) return;
    watchedHosts.set(root.host, root);
    watchedRoots.push(new WeakRef(root));
  };
  // The watched roots still alive; the dead ones leave the list.
  var liveWatched = function () {
    var live = [];
    watchedRoots = watchedRoots.filter(function (ref) {
      var root = ref.deref();
      if (root) live.push(root);
      return !!root;
    });
    return live;
  };
  // The top of each watched tree still alive and out of the document: the root of its composed
  // tree, which a script holding any node of it reaches. Dead ones leave the list.
  var detachedTops = function () {
    var tops = [];
    watchedTrees = watchedTrees.filter(function (ref) {
      var node = ref.deref();
      if (!node) return false;
      var top = node.getRootNode({ composed: true });
      if (top !== document && tops.indexOf(top) === -1) tops.push(top);
      return true;
    });
    return tops;
  };
  watch(document);

  // The document, every open shadow root under it (querySelectorAll does not pierce them), the
  // closed ones Rust handed to watch(), and each watched tree out of the document with the open
  // roots under it, its top's own included. The document's walk is bounded so a huge page
  // cannot stall a sweep. A detached tree's walk fails the sweep instead when it runs out: such
  // a tree is reachable only through the script that received it, and a sweep that left part
  // of it unseen would let that script read a field raw. The walk of a detached tree also
  // observes its top, which the page may have removed or changed since the last sweep.
  var MAX_SWEEP_ELEMENTS = 50000;
  var sweepRoots = function () {
    var roots = [document].concat(liveWatched());
    var pending = [document];
    var visited = 0;
    while (pending.length && visited < MAX_SWEEP_ELEMENTS) {
      var all = queryAll(pending.pop(), "*");
      for (var i = 0; i < all.length && visited < MAX_SWEEP_ELEMENTS; i++, visited++) {
        var open = all[i].shadowRoot;
        if (open && roots.indexOf(open) === -1) { roots.push(open); pending.push(open); observeTypes(open); }
      }
    }
    var trees = detachedTops();
    var inTrees = 0;
    while (trees.length) {
      var top = trees.pop();
      if (roots.indexOf(top) === -1) roots.push(top);
      observeTypes(top);
      var found = queryAll(top, "*");
      var each = top instanceof Element ? [top].concat(Array.prototype.slice.call(found)) : found;
      inTrees += each.length;
      if (inTrees > MAX_SWEEP_ELEMENTS) throw new Error("a detached tree a script can reach has more than " + MAX_SWEEP_ELEMENTS + " elements, too many to sweep");
      for (var j = 0; j < each.length; j++) {
        var shadow = each[j].shadowRoot;
        if (shadow && roots.indexOf(shadow) === -1) { roots.push(shadow); trees.push(shadow); observeTypes(shadow); }
      }
    }
    return roots;
  };
  sweepRoots();

  globalThis.__clawSeal = {
    version: 1,
    kind: kind,
    valueEquals: function (el, expected) { return rawValue(el) === expected; },
    attributeEquals: function (el, name, expected) { return attr(el, name) === expected; },
    rawLength: function (el) { return String(rawValue(el) || "").length; },
    /** Raw values of every sensitive field in this document and its shadow roots. */
    sensitiveValues: function () {
      var out = [];
      sweepRoots().forEach(function (root) {
        var found = Array.prototype.slice.call(queryAll(root, "input,textarea,select"));
        // A detached tree's top may be a field itself: a removed password input.
        if (root instanceof Element && isField(root)) found.push(root);
        for (var i = 0; i < found.length; i++) {
          var v = rawValue(found[i]);
          if (v && kind(found[i]) && out.indexOf(v) === -1) out.push(v);
        }
      });
      return out;
    },
    watch: watch,
    detachedTops: detachedTops,
    /** The oldest host the journal reported and Rust has not taken yet, or null. */
    takeHost: function () { return pendingHosts.length ? pendingHosts.shift() : null; },
  };
  return globalThis.__clawSeal.version;
})(__RULES__)
