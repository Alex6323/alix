export function createAugment({
  api,
  post,
  rememberLaunch,
  rerender,
  applyStudy,
  workingText,
  backendName,
  timers,
  ui,
}) {
  const {
    chip,
    confirm: confirmUser,
    deckEl,
    document: doc,
    el,
    headerBreadcrumb,
    histEl,
    legend,
    menuWrap,
    scoreEl,
    stage,
  } = ui;
  let data = null;
  let poll = null;
  const ticked = new Set();

  function isOpen() {
    return data !== null;
  }

  function isPolling() {
    return poll !== null;
  }

  // ── AI augment (the picker's "Augment a" action, decks only) ────────────────
  // Reports what a deck's augmentation cache holds, fills the gaps (a costed
  // background call, polled like the exam), and removes. Generation is one job at
  // a time; the page polls /api/augment while it runs.
  function open(deck) {
    rememberLaunch(deck);
    ticked.clear();
    return api("/api/augment/open", post({ deck })).then(d => {
      if (d && d.rows) { data = d; rerender(); }
    });
  }
  function close() {
    stopAugmentPoll();
    ticked.clear();
    return api("/api/augment/close", post({})).then(s => { data = null; applyStudy(s); return s; });
  }
  // A target card's own guidance input text, read when its Generate (or the
  // batch) is clicked.
  function augGuidance(kind) {
    const i = doc.querySelector(`.aug-guide-input[data-kind="${kind}"]`);
    return i ? i.value.trim() : "";
  }
  function augmentGenerate(target) {
    if (data.busy) return;
    const body = { targets: [{ target, with: augGuidance(target) || null }] };
    api("/api/augment/generate", post(body)).then(d => {
      data = d; refreshAugment(); if (d.busy) startAugmentPoll();
    });
  }
  // The footer's "Generate selected" action: every ticked gap-fill kind in one
  // batch, each carrying its own card's guidance. The server runs them one at a
  // time and reports queued/done/failed as it goes (polled the same way as a
  // single-target generate).
  function augmentGenerateSelected() {
    if (data.busy || !ticked.size) return;
    const targets = Array.from(ticked).map(t => ({ target: t, with: augGuidance(t) || null }));
    ticked.clear();
    api("/api/augment/generate", post({ targets })).then(d => {
      data = d; refreshAugment(); if (d.busy) startAugmentPoll();
    });
    refreshAugment();
  }
  function augmentRemove(target, topology) {
    api("/api/augment/remove", post({ target, topology: topology || null }))
      .then(d => { data = d; refreshAugment(); });
  }
  // A cheap fingerprint of the batch bookkeeping (queued/done/failed target
  // kinds) so the poll can tell a batch transition (e.g. a no-gap target
  // draining straight to "done", or the last one finishing) from a steady
  // "same busy target still running" tick, even when `busy` itself doesn't change.
  function augBatchSig(d) {
    return JSON.stringify([d.queued, d.done, (d.failed || []).map(f => f.target)]);
  }
  function startAugmentPoll() {
    stopAugmentPoll();
    poll = timers.setInterval(() => {
      api("/api/augment").then(d => {
        const prev = data;
        data = d;
        if (!d.busy) stopAugmentPoll();
        // Re-render when the in-flight state changed (a job started or finished, so
        // the coverage bars move) or the batch queue/done/failed sets changed;
        // otherwise tick the elapsed counter in place so the spinner's entrance
        // animation doesn't restart (flicker).
        if (!prev || prev.busy !== d.busy || prev.error !== d.error || augBatchSig(prev) !== augBatchSig(d)) refreshAugment();
        else {
          const e = doc.querySelector(".exam-elapsed");
          if (e) { e.innerHTML = ""; e.appendChild(el("span", "dot"));
                   e.appendChild(doc.createTextNode(augElapsedText(d))); }
        }
      });
    }, 600);
  }
  function stopAugmentPoll() { if (poll) { timers.clearInterval(poll); poll = null; } }
  function augElapsedText(d) {
    return workingText(d.elapsed || 0);
  }
  // The in-flight spinner cell (reuses the exam's pulsing dot + elapsed styling).
  function augBusyCell(d) {
    const sp = el("span", "exam-elapsed");
    sp.appendChild(el("span", "dot"));
    sp.appendChild(doc.createTextNode(augElapsedText(d)));
    return sp;
  }
  // ── AUG_INFO: static, client-only reference content for the augment cards ──
  // A plain description plus a neutral BEFORE -> AFTER preview per target kind,
  // so a user can see what an augmentation actually does before spending tokens
  // on it. Never sent by the server: d.rows only carries coverage counts.
  // `before`/`after` each fill a box element with the small, fixed preview.
  const AUG_INFO = {
    choices: {
      title: "Choices",
      desc: "Turns a card into a multiple-choice question: adds three plausible distractors alongside the answer.",
      hint: "e.g. use common misconceptions",
      before(b) {
        b.appendChild(el("div", "aug-ba-q", "Q: What is the capital of Australia?"));
        b.appendChild(el("div", "aug-ba-a", "A: Canberra"));
      },
      after(b) {
        b.appendChild(el("div", "aug-ba-q", "Q: What is the capital of Australia?"));
        const chips = el("div", "aug-ba-chips");
        ["Sydney", "Melbourne", "Perth"].forEach(name => chips.appendChild(el("span", "aug-ba-chip", name)));
        chips.appendChild(el("span", "aug-ba-chip good", "Canberra ✓"));
        b.appendChild(chips);
      },
    },
    notes: {
      title: "Notes",
      desc: "Attaches a short “why it matters” note that appears under the answer after you reveal.",
      hint: "e.g. add a mnemonic",
      before(b) {
        b.appendChild(el("div", "aug-ba-q", "Q: What is a leap year?"));
        b.appendChild(el("div", "aug-ba-a", "A: A year with 366 days (an extra day in February)."));
      },
      after(b) {
        b.appendChild(el("div", "aug-ba-q", "Q: What is a leap year?"));
        b.appendChild(el("div", "aug-ba-a", "A: A year with 366 days (an extra day in February)."));
        b.appendChild(el("div", "aug-ba-note", "Note: years divisible by 100 are not leap years unless also divisible by 400."));
      },
    },
    questions: {
      title: "Questions",
      desc: "Rewrites a card's question into a few fresh phrasings, so you read and answer it each time instead of just recognising the wording.",
      hint: "e.g. vary the angle, not just the wording",
      badge: "3 phrasings, 1 answer",
      before(b) {
        b.appendChild(el("div", "aug-ba-q", "Q: What is the freezing point of water in Celsius?"));
        b.appendChild(el("div", "aug-ba-a", "A: 0"));
      },
      after(b) {
        const ul = el("ul", "aug-ba-list");
        ["What is the freezing point of water in Celsius?",
         "At what Celsius temperature does water freeze?",
         "Water turns to ice at what temperature (in C)?"].forEach(q => ul.appendChild(el("li", null, q)));
        b.appendChild(ul);
        b.appendChild(el("div", "aug-ba-a", "→ same answer: 0"));
      },
    },
    keypoints: {
      title: "Key points",
      desc: "Distils a long answer into the few bullet points a good response must cover.",
      hint: "e.g. at most three points",
      before(b) {
        b.appendChild(el("div", "aug-ba-prose",
          "Photosynthesis lets plants turn sunlight, water, and carbon dioxide into glucose, releasing oxygen as a by-product."));
      },
      after(b) {
        const ul = el("ul", "aug-ba-list");
        ["Inputs: sunlight, water, CO2", "Output: glucose (stored energy)", "By-product: oxygen"]
          .forEach(pt => ul.appendChild(el("li", null, pt)));
        b.appendChild(ul);
      },
    },
    format: {
      title: "Formatting",
      desc: "Rewrites a plain-prose answer into clean line breaks and formatting, without changing the wording.",
      hint: "e.g. prefer numbered steps",
      before(b) {
        b.appendChild(el("div", "aug-ba-prose",
          "Order of operations: parentheses, then exponents, then multiply and divide, then add and subtract."));
      },
      after(b) {
        b.appendChild(el("div", "aug-ba-a", "Order of operations:"));
        const code = el("div", "aug-ba-code");
        ["1. Parentheses", "2. Exponents", "3. Multiply / Divide", "4. Add / Subtract"]
          .forEach(line => code.appendChild(el("div", null, line)));
        b.appendChild(code);
      },
    },
    icon: {
      title: "Icon",
      desc: "Draws a small abstract emblem for this workspace, shown on its picker row.",
      hint: "e.g. a compass rose, flat and minimal",
      before(b) {
        const chips = el("div", "aug-ba-chips");
        chips.appendChild(el("span", "aug-ba-chip", "› My workspace"));
        b.appendChild(chips);
        b.appendChild(el("div", "aug-ba-meta", "a plain chevron"));
      },
      after(b) {
        const chips = el("div", "aug-ba-chips");
        chips.appendChild(el("span", "aug-ba-chip good", "◈ My workspace"));
        b.appendChild(chips);
        b.appendChild(el("div", "aug-ba-meta", "its own emblem"));
      },
    },
    topology: {
      title: "Order",
      desc: "Orders your cards foundations first, so alix teaches the prerequisites before what builds on them.",
      hint: "e.g. by era, or north to south (also names the path)",
      before(b) {
        const chips = el("div", "aug-ba-chips");
        ["Addition", "Multiplication", "Exponents"].forEach(name => chips.appendChild(el("span", "aug-ba-chip", name)));
        b.appendChild(chips);
        b.appendChild(el("div", "aug-ba-meta", "unordered, reviewed at random"));
      },
      after(b) {
        const chain = el("div", "aug-ba-chips");
        ["Addition", "Multiplication", "Exponents"].forEach((name, i) => {
          if (i > 0) chain.appendChild(el("span", "aug-ba-arrow-sm", "→"));
          chain.appendChild(el("span", "aug-ba-chip good", name));
        });
        b.appendChild(chain);
        b.appendChild(el("div", "aug-ba-meta", "foundations first"));
      },
    },
  };
  // One BEFORE or AFTER box: a label (+ optional badge on the same line, e.g.
  // "questions"'s "3 phrasings, 1 answer"), then the kind's preview content.
  function augBaBox(cls, label, accent, badge, fill) {
    const box = el("div", cls);
    const head = el("div", "aug-ba-head");
    head.appendChild(el("span", "aug-ba-label" + (accent ? " accent" : ""), label));
    if (badge) head.appendChild(el("span", "aug-ba-badge", badge));
    box.appendChild(head);
    fill(box);
    return box;
  }
  // The BEFORE -> AFTER preview row for a target kind, from AUG_INFO. A kind
  // missing from AUG_INFO (shouldn't happen, the six targets are fixed) just
  // renders empty boxes rather than throwing.
  function augBeforeAfter(kind) {
    const info = AUG_INFO[kind];
    const wrap = el("div", "aug-ba");
    wrap.appendChild(augBaBox("aug-before", "BEFORE", false, null, b => { if (info) info.before(b); }));
    wrap.appendChild(el("div", "aug-arrow", "→"));
    wrap.appendChild(augBaBox("aug-after", "AFTER", true, info && info.badge, b => { if (info) info.after(b); }));
    return wrap;
  }
  // A card's own compact guidance input. Its kind-specific example placeholder
  // doubles as the hint that a steer is possible here at all — the reason the
  // input sits on every card instead of once in the footer.
  function augGuideInput(kind) {
    const gi = el("input", "aug-guide-input");
    gi.type = "text";
    gi.dataset.kind = kind;
    const info = AUG_INFO[kind];
    gi.placeholder = "guidance (optional)" + (info && info.hint ? ": " + info.hint : "");
    // Backspace edits the guidance, it doesn't leave the view (like the picker filter).
    gi.addEventListener("keydown", (e) => { if (e.key === "Backspace") e.stopPropagation(); });
    return gi;
  }
  // The shared card shell: title (AUG_INFO's, falling back to the server's
  // row.label) + description on the left, the caller's coverage/action markup
  // on the right, then the card's guidance input and the before/after preview.
  function augCardShell(row, right) {
    const info = AUG_INFO[row.kind];
    // A green border marks a target that is already fully generated (or, for
    // topology, has at least one named topology), so you can see at a glance what
    // this deck already holds versus what still needs generating.
    const done = row.kind === "topology" ? row.items.length > 0 : row.eligible > 0 && row.covered >= row.eligible;
    const card = el("div", "aug-card" + (done ? " done" : ""));
    const head = el("div", "aug-card-head");
    const meta = el("div", "aug-card-info");
    meta.appendChild(el("div", "aug-card-title", (info && info.title) || row.label));
    if (info) meta.appendChild(el("div", "aug-card-desc", info.desc));
    head.appendChild(meta);
    head.appendChild(right);
    card.appendChild(head);
    card.appendChild(augGuideInput(row.kind));
    card.appendChild(augBeforeAfter(row.kind));
    return card;
  }
  // A per-target card: an optional batch-select checkbox (only while there's an
  // actual gap to fill), the coverage count, a status derived from the current
  // poll (busy/queued/failed/done take precedence over the plain buttons), and
  // Generate (fills the gap) + Remove.
  function augCardRow(d, row) {
    const full = row.eligible && row.covered >= row.eligible;
    const hasGap = row.eligible > row.covered;
    // A tick left over from before a gap closed (e.g. a direct per-card Generate
    // while it was also ticked) would otherwise silently inflate the footer's
    // selected count with no checkbox on screen to explain it.
    if (!hasGap) ticked.delete(row.kind);
    const right = el("div", "aug-actions");
    if (hasGap) {
      const cb = el("input", "aug-check");
      cb.type = "checkbox";
      cb.title = "Select for batch generate";
      cb.checked = ticked.has(row.kind);
      cb.disabled = !!d.busy;
      cb.addEventListener("change", () => {
        if (cb.checked) ticked.add(row.kind); else ticked.delete(row.kind);
        renderAugLegend(); syncAugTools();
      });
      right.appendChild(cb);
    }
    right.appendChild(el("span", "aug-count" + (full ? " full" : ""), row.covered + "/" + row.eligible));
    const failedEntry = (d.failed || []).find(f => f.target === row.kind);
    const queued = (d.queued || []).includes(row.kind);
    const justDone = (d.done || []).includes(row.kind);
    if (row.busy) right.appendChild(augBusyCell(d));
    else if (queued) right.appendChild(el("span", "aug-status queued", "queued"));
    else {
      if (failedEntry) {
        const err = el("span", "aug-status failed", failedEntry.error);
        err.title = failedEntry.error;
        right.appendChild(err);
      } else if (justDone && !hasGap) right.appendChild(el("span", "aug-status done", "done ✓"));
      const gen = el("button", "aug-btn", full ? "Complete" : "Generate");
      gen.disabled = !!d.busy || !row.eligible || !!full;
      gen.addEventListener("click", () => augmentGenerate(row.kind));
      right.appendChild(gen);
      if (row.covered > 0) {
        const rm = el("button", "aug-btn ghost", "Remove");
        rm.disabled = !!d.busy;
        rm.addEventListener("click", () => augmentRemove(row.kind, null));
        right.appendChild(rm);
      }
    }
    return augCardShell(row, right);
  }
  // The topology card: its named topologies (each removable) + Add (uses guidance).
  function augTopologyRow(d, row) {
    const list = el("span", "aug-topos");
    if (!row.items.length) list.appendChild(el("span", "aug-none", "none yet"));
    for (const name of row.items) {
      const pill = el("span", "aug-topo");
      pill.appendChild(el("span", null, name));
      if (!d.busy) {
        const x = el("button", "aug-x", "✕");
        x.title = "Remove this order";
        x.addEventListener("click", () => augmentRemove("topology", name));
        pill.appendChild(x);
      }
      list.appendChild(pill);
    }
    const right = el("div", "aug-actions");
    // The topology card is batchable now that pedagogical order is the default: tick it and
    // "Generate selected" produces the default path (same-name paths replace, so
    // it never piles up duplicates). Always selectable, since there is no gap.
    const cb = el("input", "aug-check");
    cb.type = "checkbox";
    cb.title = "Select for batch generate";
    cb.checked = ticked.has("topology");
    cb.disabled = !!d.busy;
    cb.addEventListener("change", () => {
      if (cb.checked) ticked.add("topology"); else ticked.delete("topology");
      renderAugLegend(); syncAugTools();
    });
    right.appendChild(cb);
    right.appendChild(list);
    if (row.busy) right.appendChild(augBusyCell(d));
    else if ((d.queued || []).includes("topology")) right.appendChild(el("span", "aug-status queued", "queued"));
    else {
      // A failed topology generation lands in d.failed like any other target;
      // show it here (d.error stays null for per-target failures) so an Add that
      // errors isn't silent.
      const failedEntry = (d.failed || []).find(f => f.target === "topology");
      if (failedEntry) {
        const err = el("span", "aug-status failed", failedEntry.error);
        err.title = failedEntry.error;
        right.appendChild(err);
      }
      // "Generate" like the other targets when there are none yet; "Generate
      // another" once some exist, since a deck can hold several named topologies.
      const add = el("button", "aug-btn", row.items.length ? "Generate another" : "Generate");
      add.disabled = !!d.busy;
      add.addEventListener("click", () => augmentGenerate("topology"));
      right.appendChild(add);
    }
    return augCardShell(row, right);
  }
  // The workspace icon card: always regenerable (a fresh draw replaces the old
  // emblem), so unlike a gap-fill target it stays tickable and enabled when
  // covered; the green border still marks "has one".
  function augIconRow(d, row) {
    const right = el("div", "aug-actions");
    const cb = el("input", "aug-check");
    cb.type = "checkbox";
    cb.title = "Select for batch generate";
    cb.checked = ticked.has("icon");
    cb.disabled = !!d.busy;
    cb.addEventListener("change", () => {
      if (cb.checked) ticked.add("icon"); else ticked.delete("icon");
      renderAugLegend(); syncAugTools();
    });
    right.appendChild(cb);
    if (row.busy) right.appendChild(augBusyCell(d));
    else if ((d.queued || []).includes("icon")) right.appendChild(el("span", "aug-status queued", "queued"));
    else {
      const failedEntry = (d.failed || []).find(f => f.target === "icon");
      if (failedEntry) {
        const err = el("span", "aug-status failed", failedEntry.error);
        err.title = failedEntry.error;
        right.appendChild(err);
      } else if ((d.done || []).includes("icon")) right.appendChild(el("span", "aug-status done", "done ✓"));
      const gen = el("button", "aug-btn", row.covered ? "Regenerate" : "Generate");
      gen.disabled = !!d.busy;
      gen.addEventListener("click", () => augmentGenerate("icon"));
      right.appendChild(gen);
    }
    return augCardShell(row, right);
  }
  // Whether a row can join the batch: gap-fill targets need an actual gap;
  // topology and the icon are always re-runnable.
  function augTickable(row) {
    if (row.kind === "topology" || row.kind === "icon") return true;
    return row.eligible > row.covered;
  }
  // Keeps the on-page Select all button's label honest after a manual tick,
  // without re-rendering the cards (ticking must never repaint the page).
  function syncAugTools() {
    const d = data;
    const btn = doc.querySelector(".aug-tools button");
    if (!d || !btn) return;
    const tickable = d.rows.filter(augTickable).map(r => r.kind);
    const allOn = tickable.length > 0 && tickable.every(k => ticked.has(k));
    btn.textContent = allOn ? "Clear selection" : "Select all";
  }
  // Builds the augment body (select-all + target cards + the cost footer) into
  // `wrap`. Shared by the first mount and the in-place refresh.
  function augFillContent(wrap) {
    const d = data;
    if (d.error) wrap.appendChild(el("div", "exam-error", "⚠ " + d.error));
    const tools = el("div", "aug-tools");
    const tickable = d.rows.filter(augTickable).map(r => r.kind);
    const allOn = tickable.length > 0 && tickable.every(k => ticked.has(k));
    const sa = el("button", "aug-btn ghost", allOn ? "Clear selection" : "Select all");
    sa.disabled = !!d.busy || !tickable.length;
    sa.addEventListener("click", () => {
      const on = tickable.length > 0 && tickable.every(k => ticked.has(k));
      if (on) ticked.clear(); else tickable.forEach(k => ticked.add(k));
      refreshAugment();
    });
    tools.appendChild(sa);
    wrap.appendChild(tools);
    for (const row of d.rows)
      wrap.appendChild(row.kind === "topology" ? augTopologyRow(d, row)
        : row.kind === "icon" ? augIconRow(d, row) : augCardRow(d, row));
    const foot = el("div", "aug-foot");
    foot.appendChild(el("div", "aug-cost",
      `Generating runs ${backendName()} and costs tokens. It fills only the cards a target is missing.`));
    wrap.appendChild(foot);
  }
  function render() {
    const d = data;
    headerBreadcrumb();
    deckEl.textContent = "augment · " + d.deck;
    histEl.textContent = d.cards + (d.cards === 1 ? " card" : " cards");
    scoreEl.innerHTML = "";
    menuWrap.style.display = "none";
    const wrap = el("div", "aug");
    augFillContent(wrap);
    stage.appendChild(wrap);
    renderAugLegend();
  }
  // Updates the augment screen in place after an action (generate/remove/poll):
  // rebuild the cards + footer inside the existing container so the entrance
  // animation never replays and the scroll position (and any typed guidance,
  // per card, plus its focus) is kept. Falls back to a full render if the
  // screen is not mounted yet.
  function refreshAugment() {
    const existing = stage.querySelector(".aug");
    if (!existing) { rerender(); return; }
    const scrollTop = existing.scrollTop;
    const typed = {};
    let focused = null;
    existing.querySelectorAll(".aug-guide-input").forEach(i => {
      if (i.value) typed[i.dataset.kind] = i.value;
      if (i === doc.activeElement) focused = i.dataset.kind;
    });
    existing.innerHTML = "";
    augFillContent(existing);
    existing.querySelectorAll(".aug-guide-input").forEach(i => {
      if (typed[i.dataset.kind]) i.value = typed[i.dataset.kind];
      if (i.dataset.kind === focused) {
        i.focus();
        i.setSelectionRange(i.value.length, i.value.length);
      }
    });
    existing.scrollTop = scrollTop;
    renderAugLegend();
  }
  // Rebuilds only the footer's selection controls (work line + Generate selected
  // + Remove all + Close). Ticking a checkbox calls this instead of a full
  // render, so the screen's entrance animation never replays and scroll is kept.
  function renderAugLegend() {
    const d = data;
    if (!d) return;
    legend.innerHTML = "";
    const tickedCount = ticked.size;
    if (tickedCount > 0) {
      let work = 0;
      for (const row of d.rows) if (ticked.has(row.kind)) work += row.kind === "topology" || row.kind === "icon" ? 1 : Math.max(0, row.eligible - row.covered);
      legend.appendChild(el("span", "aug-work",
        `will run ~${work} generation${work === 1 ? "" : "s"} across ${tickedCount} target${tickedCount === 1 ? "" : "s"}`));
    }
    // Destructive first and Close last (the dialog convention), so the
    // most-clicked chip never neighbors "Remove all".
    const rmAll = chip("Remove all", "", () => {
      if (confirmUser("Remove every augmentation for this deck?")) augmentRemove("all");
    });
    rmAll.disabled = !!d.busy;
    const genSel = chip(`Generate selected (${tickedCount})`, "primary", augmentGenerateSelected);
    genSel.disabled = tickedCount === 0 || !!d.busy;
    chip("Close", "", close, "esc");
  }


  function handleKey(event) {
    if (!data) return false;
    if (event.key === "Escape" || event.key === "Backspace") {
      event.preventDefault();
      close();
    }
    return true;
  }

  return {
    close,
    data: () => data,
    handleKey,
    isOpen,
    isPolling,
    open,
    render,
  };
}
