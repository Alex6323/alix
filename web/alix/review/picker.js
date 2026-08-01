export function createPicker({
  api,
  post,
  sessionStorage,
  currentState,
  isBrowsing,
  examIsOpen,
  augmentIsOpen,
  walkIsOpen,
  tutorIsOpen,
  applyStudy,
  openWalk,
  openBrowse,
  startExam,
  openAugment,
  notice,
  timers,
  ui,
}) {
  const {
    barFilter,
    chip,
    clearLegendSides,
    deckEl,
    document: doc,
    el,
    headerNone,
    headerSearch,
    histEl,
    hit,
    label,
    legend,
    legendLeft,
    legendRight,
    masteredBtn,
    menuWrap,
    navRefresh,
    replayLogo,
    scoreEl,
    setMenuContext,
    stage,
    window: win,
  } = ui;
  let selectEsc = null;
  let lastWorkspace = null;
  let drawerSel = { deck: null, topology: null, region: null };
  let topoCache = {};
  let cramOn = false;
  let focusedEl = null;
  let iconNonce = 0;
  const drawerDurationMs = 170;
  const drawerEasing = "cubic-bezier(0.4, 0, 0.2, 1)";
  let keys = {
    up: [{ k: "k", ctrl: false }],
    down: [{ k: "j", ctrl: false }],
    open: [{ k: "l", ctrl: false }],
    back: [{ k: "h", ctrl: false }],
    filter: [{ k: "/", ctrl: false }, { k: "f", ctrl: true }],
    mastered: [{ k: "m", ctrl: false }],
    depth: [{ k: "v", ctrl: false }],
    recognize: [{ k: "1", ctrl: false }],
    recall: [{ k: "2", ctrl: false }],
    reconstruct: [{ k: "3", ctrl: false }],
    cram: [{ k: "c", ctrl: false }],
  };

  function setKeys(next) {
    keys = Object.assign(keys, next || {});
  }

  function rememberLaunch(name) {
    if (name) sessionStorage.setItem("alix.lastDeck", name);
  }

  function isWalk(next) {
    return !!next && next.kind === "walk";
  }

  function select(name, topology, region, depth, cram) {
    rememberLaunch(name);
    return api("/api/select", post({
      deck: name,
      topology: topology || null,
      region: region || null,
      depth: depth || null,
      cram: !!cram,
    })).then((next) => {
      if (isWalk(next)) openWalk(next);
      else applyStudy(next);
      return next;
    }).catch(() => notice("could not start the session: the server log has details"));
  }

  function browse(item, workspaceName) {
    rememberLaunch(item.name);
    lastWorkspace = workspaceName || null;
    return openBrowse(item);
  }

  function focusedRowName() {
    const row = focusedEl;
    return (row && ((row._item && row._item.name) || (row._open && row._open.name))) || null;
  }

  function refresh() {
    iconNonce = Date.now();
    replayLogo();
    return render();
  }

  navRefresh.addEventListener("click", refresh);

  function treeGuides(prefix) {
    const box = el("span", "tree");
    for (let i = 0; i + 3 <= prefix.length; i += 3) {
      const c = prefix[i];
      const cls = c === "│" ? "line" : c === "├" ? "tee" : c === "└" ? "ell" : "empty";
      box.appendChild(el("span", `guide ${cls}`));
    }
    return box;
  }

  function paintHeatCell(cell, tier) {
    cell.classList.add(tier === "untouched" ? "empty" : tier);
  }

  let lastDecksSignature = "";
  // A comparable snapshot of a decks payload that ignores the volatile
  // "<n><unit> ago" tokens (src/time.rs humanize_ms: e.g. "8s ago", "3m ago"),
  // which tick every second on their own and would otherwise make every
  // payload look changed. `days_left` (deadline chips) is NOT touched here —
  // its rollover is a real change (the chip's urgency tier can flip) worth
  // repainting for.
  function catalogSignature(data) {
    return JSON.stringify(data).replace(/\d+[smhdw] ago/g, "\u0000 ago");
  }
  const idleInSelect = () =>
    currentState() && currentState().phase === "select" && !isBrowsing() && !examIsOpen() && !augmentIsOpen() && !walkIsOpen() && !tutorIsOpen();
  win.addEventListener("focus", async () => {
    if (!idleInSelect()) return;
    // An opportunistic re-scan stays quiet on failure too; the visible error
    // state belongs to the deliberate loads (initial, refresh, retry).
    const fresh = await api("/api/decks").catch(() => null);
    if (!fresh) return;
    // Re-check after the await: the user may have started something meanwhile.
    if (idleInSelect() && catalogSignature(fresh) !== lastDecksSignature) {
      render(fresh);
    }
  });

  // The deck-selection screen, mirroring the terminal picker. Three sections —
  // Workspaces (each with its last-progress time), Recent loose decks, and
  // Folders — and single-launch: click a deck to start it (a trace walks, a deck
  // reviews, an exam-due deck sits its exam) or open a workspace/folder to drill
  // into its unlock dependency tree. 🔒 exam locked (still drillable) · 🕒 nothing due ·
  // mastered 🎉 decks live in the Mastered window (m). The filter searches every
  // loose deck.
  // `preloaded`, when given, skips the GET — used after a POST (e.g. setting a
  // workspace deadline) whose response is already the refreshed decks payload,
  // so the round trip stays singular instead of following up with a fetch.
  async function render(preloaded) {
    deckEl.textContent = "";
    histEl.textContent = "";
    scoreEl.innerHTML = "";
    menuWrap.style.display = "";
    setMenuContext("picker");
    // Drop cached topology heatmaps so the drawer reflects progress from any
    // session just finished (the strengths are recomputed on next focus).
    topoCache = {};

    let data = preloaded;
    if (!data) {
      stage.innerHTML = "";
      stage.appendChild(el("div", "msg", "loading decks…"));
      try {
        data = await api("/api/decks");
      } catch {
        stage.innerHTML = "";
        const wrap = el("div", "select");
        wrap.appendChild(el("div", "lede", "choose decks to study"));
        wrap.appendChild(el("div", "msg", "Couldn't read the decks folder."));
        const retry = el("button", "chip primary", "Retry");
        retry.addEventListener("click", () => render());
        wrap.appendChild(retry);
        stage.appendChild(wrap);
        return;
      }
      stage.innerHTML = "";
    }
    lastDecksSignature = catalogSignature(data);
    const workspaces = data.workspaces || [];
    const recent = data.recent || [];
    const folders = data.folders || [];
    if (!workspaces.length && !recent.length && !folders.length) {
      const wrap = el("div", "select");
      wrap.appendChild(el("div", "lede", "choose decks to study"));
      wrap.appendChild(el("div", "msg", "No decks found. Add .txt decks to your decks folder."));
      stage.appendChild(wrap);
      return;
    }

    // Every mastered (exam-passed) deck across the catalog, for the m window.
    const mastered = recent.filter(d => d.mastered);
    for (const g of workspaces.concat(folders)) for (const m of g.members) if (m.mastered) mastered.push(m);

    // Can a row be started now, at *any* depth? Drilling is never gated by the
    // prerequisite lock (only the exam is — an exam-locked deck's `examable` is
    // false), so a gated view refuses only a deck with nothing due at any depth.
    // `reviewable` already folds in the trace/exam-due special cases alongside
    // the per-depth due-ness, so this reads as "any depth (or the trace/exam)
    // is startable" — the gate for the ▾ split button. The Mastered window is
    // ungated — a finished deck can be reopened to cram or re-examine.
    const canStart = (it, gated) => it.reviewable || !gated;

    // Maps a depth name to its own due-ness field (`picker::DeckStatus`'s
    // per-depth split), so each depth chip — and the plain Learn button, which
    // targets the deck's own last-used depth — gates on its own honest signal
    // rather than "any depth" (recall-settled must not enable a Recall chip
    // just because Reconstruct is due).
    const DEPTHS = ["recognize", "recall", "reconstruct"]; // menu/keys order — 1/2/3
    const DEPTH_FIELD = { recognize: "reviewable_recognize", recall: "reviewable_recall", reconstruct: "reviewable_reconstruct" };
    const canStartAt = (it, gated, depth) => it[DEPTH_FIELD[depth]] || !gated;
    // Recognize is pick-only: it can only run on a deck with cached choice
    // distractors (`can_recognize`) — an un-augmented deck greys it out even under
    // cram (which re-serves recognized cards). Recall/Reconstruct are never gated
    // on augmentation.
    const canDoDepth = (it, depth) => depth !== "recognize" || !!it.can_recognize;

    // Rows that carry the depth split (the Learn ▾ chip and its v key): a deck —
    // not a workspace/folder — that isn't exam-primary, has a remembered depth,
    // and isn't a trace (walked — depths don't apply).
    const hasDepthSplit = (row) => !!(row && row._item && !row._open
      && row._item.state !== "examdue" && row._item.last_depth && !row._item.is_trace);

    // The plain Learn/primary button's gate: a trace always walks and an
    // exam-due deck's primary is its exam (both non-depth, via `canStart`);
    // otherwise it reviews at the deck's own last-used depth.
    const canStartPrimary = (it, gated) =>
      (it.is_trace || it.state === "examdue") ? canStart(it, gated)
        : (canDoDepth(it, it.last_depth) && canStartAt(it, gated, it.last_depth));

    // Launch one deck/member. An exam-due deck sits its exam; a trace will walk
    // once the web hosts walks (for now it reviews its explain cards — the single
    // place to change). Launching inside a workspace remembers it so leaving the
    // session returns here.
    function launch(it, wsName, gated) {
      if (!canStartPrimary(it, gated)) return;
      lastWorkspace = wsName || null;
      // A trace's primary action is always the WALK (its exam is reached via the
      // "Take exam" button, or the walk's capstone). An exam-due fact deck sits its
      // exam when available (sourced + prerequisites passed); else it reviews.
      if (!it.is_trace && it.state === "examdue" && it.examable) startExam(it.name);
      else {
        // Apply the focus drawer's topology/region pick, but only when it belongs
        // to the deck being launched (the drawer follows the focused row).
        const sel = drawerSel.deck === it.name ? drawerSel : {};
        select(it.name, sel.topology, sel.region);
      }
    }

    // Launch at an explicit depth (from the split Learn button's ▾ menu), same
    // drawer-scope rules as `launch`, but never routes to the exam — picking a
    // depth always means "review", gated on that depth's own due-ness.
    function launchDepth(it, wsName, gated, depth) {
      if (!canDoDepth(it, depth)) return;
      if (!cramOn && !canStartAt(it, gated, depth)) return;
      lastWorkspace = wsName || null;
      const sel = drawerSel.deck === it.name ? drawerSel : {};
      select(it.name, sel.topology, sel.region, depth, cramOn);
    }

    // The workspace's emblem if it has one, else the chevron. An SVG renders as a
    // theme-tinted mask (so it follows the active theme); a raster renders as-is.
    function rowIcon(grp) {
      if (grp.icon) {
        // The nonce (bumped by ⟳/r) makes a regenerated emblem's stable URL
        // look new to the browser cache; 0 = untouched URLs on a fresh load.
        const src = iconNonce ? `${grp.icon}?v=${iconNonce}` : grp.icon;
        if (grp.icon_svg) {
          const span = el("span", "icon mask");
          span.style.webkitMaskImage = `url("${src}")`;
          span.style.maskImage = `url("${src}")`;
          return span;
        }
        const img = doc.createElement("img");
        img.className = "icon";
        img.src = src;
        img.alt = "";
        return img;
      }
      return el("span", "open", "›");
    }

    // A workspace's deadline readout ({#deadlines}): the chip's short form and
    // the drawer's long form only differ in phrasing — both key off
    // `days_left < 0` for "past". A folder's `deadline` is always null (only a
    // real workspace has one — see catalog.rs), so callers just gate on it.
    function deadlineChipText(dl) {
      return dl.days_left < 0
        ? `🎯 was due ${dl.date}`
        : `🎯 ${dl.date} · ${dl.days_left}d · ${Math.round(100 * dl.ready / Math.max(1, dl.total))}%`;
    }
    // Urgency tier for the chip's color: silent (dim) while the date is far,
    // accent inside the last week, warn past due. Aware when it matters.
    function deadlineTier(dl) {
      if (dl.days_left < 0) return " past";
      return dl.days_left <= 7 ? " near" : "";
    }
    function deadlineLineText(dl) {
      const when = dl.days_left < 0 ? `was due ${dl.date}` : dl.date;
      return `🎯 ${when} · ${dl.ready}/${dl.total} mastered`;
    }

    // Sets, moves, or clears a workspace's deadline. The endpoint returns the
    // refreshed decks payload, so this is a single round trip — feed it straight
    // back into render rather than following up with a GET. Sets the
    // one-shot re-land marker (the same trick renderDrill's `back` uses below)
    // so the top list re-focuses this workspace row rather than the first one.
    function submitDeadline(name, date) {
      api("/api/workspace/deadline", post({ name, date }))
        .then(async (d) => {
          sessionStorage.setItem("alix.lastDeck", name);
          await render(d);
          // Put focus back on the workspace row the change was made from, so
          // set AND clear both resume keyboard flow in place.
          for (const r of deckEl.querySelectorAll(".deckrow")) {
            if (r._open && r._open.name === name) { r.focus(); break; }
          }
        })
        .catch(() => notice("could not update the deadline: the server log has details"));
    }

    // A drillable workspace/folder row (icon/chevron, opens its members).
    function openRow(grp) {
      const row = el("div", "deckrow");
      row.tabIndex = 0;
      row.appendChild(rowIcon(grp));
      // A workspace's description (its goal) sits dim under the title.
      const text = el("div", "rowtext");
      text.appendChild(el("span", "name", grp.label || grp.name));
      if (grp.description) text.appendChild(el("span", "desc", grp.description));
      row.appendChild(text);
      if (grp.path) row.appendChild(el("span", "loc", grp.path));
      // The deadline chip sits BEFORE the meta so the deck-count/ago column
      // stays vertically aligned across rows with and without a deadline.
      if (grp.deadline) {
        const dl = grp.deadline;
        row.appendChild(el("span", "deadline-chip" + deadlineTier(dl), deadlineChipText(dl)));
      }
      if (grp.meta) row.appendChild(el("span", "meta", grp.meta));
      row.addEventListener("click", () => renderDrill(grp));
      row._open = grp;
      row._search = (grp.label || grp.name).toLowerCase();
      row._default = true;
      return row;
    }

    // A single deck/member row. `gated` toggles the nothing-due gate; `wsName` is
    // the workspace to return into; `dflt` whether it shows before any filter;
    // `showKind` tags a trace (only in a drill-in, like the TUI — Recent omits it).
    function deckRow(it, gated, wsName, dflt, showKind) {
      // Drilling is never blocked by the lock, so dim only a deck with nothing to
      // launch (nothing due). A drillable locked deck stays bright but keeps 🔒.
      const dimmed = gated && !it.reviewable;
      const row = el("div", "deckrow" + (dimmed ? " dim" : ""));
      row.tabIndex = 0;
      // The dependency-tree branch prefix (├─/└─/│), drawn for workspace members
      // like the TUI; it provides the indentation, and is hidden while filtering
      // (a filtered subset is no longer a tree). An exam-locked deck has no row
      // glyph — the footer names the lock for the focused row.
      if (it.tree) row.appendChild(treeGuides(it.tree));
      row.appendChild(el("span", "name", it.label || it.name));
      if (showKind && it.is_trace) row.appendChild(el("span", "kind", "trace"));
      if (it.path) row.appendChild(el("span", "loc", it.path));
      // The highest depth with a badge (solid border = currently solid, dashed =
      // earned but lapsed — subsumption, spec {#check-matrix}); `new` corner chip
      // when any card is fresh. Both absent on workspace/folder rows (no fields).
      if (it.badge_depth) {
        const d = it.badge_depth;
        const badge = el("span", "badge-depth" + (it.badge_dotted ? " dotted" : ""), d[0].toUpperCase() + d.slice(1));
        badge.title = it.badge_dotted ? d + " badge earned, currently lapsed" : d + " badge, currently solid";
        row.appendChild(badge);
      }
      if (it.new_cards) row.appendChild(el("span", "badge-new", "new"));
      if (it.meta) row.appendChild(el("span", "meta state-" + it.state, it.meta));
      // 🕒 nothing due — at the line end with the status, so the left gutter stays
      // tree + title. (A finished deck shows its 🎉 in the badge instead.)
      if (gated && !it.reviewable && it.state !== "finished") row.appendChild(el("span", "glyph", "\u{1F552}"));
      // Click selects (focuses) the deck, opening its focus drawer, rather than
      // launching outright; Review / Enter then launches.
      row.addEventListener("click", () => row.focus());
      row._item = it; row._gated = gated; row._wsName = wsName;
      row._search = (it.label || it.name).toLowerCase();
      row._default = dflt !== false;
      return row;
    }

    // Renders a sectioned list with a filter, focus-driven primary button, and
    // keyboard navigation. `sections` is [{ title, rows }] (title null = no
    // header). `gated` is informational; `back` (Esc/h) leaves the view;
    // `allowMastered` binds the m key + a chip to the Mastered window.
    function renderList(opts) {
      selectEsc = opts.back || null;
      stage.innerHTML = ""; legend.innerHTML = "";
      focusedEl = null; // module-level (see declaration) — reset fresh each render
      let drawerEl = null; // the focus drawer under the focused deck, if it has a topology
      let closingEl = null; // a drawer mid-close (animating out before removal)
      let drawerCycle = null; // (dir)=>{} to step the drawer's region selection, when open
      let depthMenuOpen = false; // the split Learn button's ▾ menu (Recognize/Recall/Reconstruct) is open
      let deadlinePromptOpen = false; // the "Ready by…" inline date prompt is open

      const wrap = el("div", "select");
      if (opts.lede) {
        const lede = el("div", "lede", opts.lede);
        // The deadline readout rides inline behind the title (no extra line).
        if (opts.deadline) {
          const dl = opts.deadline;
          lede.appendChild(el("span", "lede-deadline" + deadlineTier(dl), deadlineLineText(dl)));
        }
        wrap.appendChild(lede);
      }
      // A workspace drill-in shows its goal (description) under the eyebrow.
      if (opts.ledeDesc) wrap.appendChild(el("div", "lede-desc", opts.ledeDesc));
      let filter;
      if (opts.headerFilter) {
        // The picker's search and Mastered jump live in the header; no in-content box.
        headerSearch();
        filter = barFilter;
        filter.value = "";
        if (opts.allowMastered && mastered.length) {
          masteredBtn.style.display = "";
          masteredBtn.onclick = renderMastered;
        } else {
          masteredBtn.style.display = "none";
        }
      } else {
        headerNone();
        filter = el("input", "deck-filter");
        filter.type = "text"; filter.autocomplete = "off";
        wrap.appendChild(filter);
      }
      filter.placeholder = opts.filterPlaceholder || "Search  ·  / or Ctrl-F";

      const lists = el("div", "lists");
      const sectionEls = [];
      for (const sec of opts.sections) {
        const header = sec.title ? el("div", "section", sec.title) : null;
        if (header) lists.appendChild(header);
        for (const r of sec.rows) lists.appendChild(r);
        sectionEls.push({ header, rows: sec.rows });
      }
      const emptyHint = el("div", "empty-hint", "No decks match.");
      lists.appendChild(emptyHint);
      wrap.appendChild(lists);
      stage.appendChild(wrap);

      const visibleRows = () =>
        Array.from(lists.querySelectorAll(".deckrow")).filter(r => r.style.display !== "none");

      // The primary button reflects the focused row: Open a workspace/folder,
      // Start/Take exam a deck (disabled when locked or nothing due).
      function syncPrimary() {
        legend.innerHTML = "";
        // Going back (Esc) gets the same footer-chip UI the sessions use for
        // Leave, pinned bottom-left; nothing at the picker's top level. While a
        // footer submenu (depth menu, the date prompt) is open, Esc means
        // "close it" via that menu's own Cancel, so Back hides to keep the key
        // meaning single.
        legendLeft.innerHTML = "";
        if (selectEsc && !depthMenuOpen && !deadlinePromptOpen) {
          chip("Back", "", () => selectEsc(), "esc", legendLeft);
        }
        const f = focusedEl;
        // The split Learn button's depth menu: temporarily swaps the whole footer
        // for Recognize/Recall/Reconstruct + Cancel, focused row's own last-used
        // depth highlighted as the primary. Closed by picking one, Cancel, or Esc.
        if (f && f._item && depthMenuOpen) {
          const it = f._item;
          // mousedown preventDefault keeps focus on the deck row (the kebab /
          // drawer-cell trick), so row nav and the menu's own Escape keep working
          // after any of these buttons is clicked — a click otherwise moves focus
          // onto a button syncPrimary() is about to destroy, stranding it on <body>.
          // The key map's per-depth bindings use the depth names themselves,
          // so keys[d] is each chip's binding (1/2/3 by default). With cram on,
          // every depth is startable — cram serves cards that aren't due.
          for (const d of DEPTHS) {
            const b = el("button", "chip" + (d === it.last_depth ? " primary" : ""), d[0].toUpperCase() + d.slice(1));
            b.appendChild(el("span", "k", label(keys[d])));
            b.disabled = !canDoDepth(it, d) || (!cramOn && !canStartAt(it, f._gated, d));
            b.addEventListener("mousedown", e => e.preventDefault());
            b.addEventListener("click", () => { depthMenuOpen = false; launchDepth(it, f._wsName, f._gated, d); });
            legend.appendChild(b);
          }
          // The cram tick-box: include cards that aren't due (a due card still
          // grades as a normal review; an early pass only re-anchors).
          const cram = el("button", "chip" + (cramOn ? " primary" : ""), (cramOn ? "☑" : "☐") + " cram");
          cram.title = "include cards that aren't due; due cards still count as normal reviews";
          cram.appendChild(el("span", "k", label(keys.cram)));
          cram.addEventListener("mousedown", e => e.preventDefault());
          cram.addEventListener("click", () => { cramOn = !cramOn; syncPrimary(); });
          legend.appendChild(cram);
          const cancel = el("button", "chip", "Cancel");
          cancel.appendChild(el("span", "k", "esc"));
          cancel.addEventListener("mousedown", e => e.preventDefault());
          cancel.addEventListener("click", () => { depthMenuOpen = false; syncPrimary(); });
          // Takes Back's slot rather than trailing the depth chips: it is the
          // same "leave this level" action Esc performs, so it stays put.
          legendLeft.appendChild(cancel);
          return;
        }
        // "Ready by…"'s inline date prompt: the same whole-footer swap as the
        // depth menu above, but the date input needs real focus to be typed
        // into — so unlike the depth menu it doesn't try to keep the row
        // focused; it owns Enter/Escape itself instead (see its keydown below).
        if (f && f._open && f._open.state === "workspace" && deadlinePromptOpen) {
          const row = f;
          const dl = f._open.deadline;
          // Closing without a change puts focus back on the row the prompt
          // came from, so keyboard flow resumes where the user left it.
          const closePrompt = () => { deadlinePromptOpen = false; syncPrimary(); row.focus(); };
          const input = el("input", "deadline-input");
          input.type = "date";
          if (dl) input.value = dl.date;
          input.addEventListener("keydown", (e) => {
            e.stopPropagation(); // owns every key while editing — the picker's
                                  // own Esc-to-leave must not fire mid-edit
            if (e.key === "Enter") { e.preventDefault(); submitDeadline(f._open.name, input.value || null); }
            else if (e.key === "Escape") { e.preventDefault(); closePrompt(); }
            else if (e.key === "c" && dl) { e.preventDefault(); submitDeadline(f._open.name, null); }
          });
          legend.appendChild(input);
          const set = el("button", "chip primary", "Set");
          set.appendChild(el("span", "k", "enter"));
          set.addEventListener("click", () => submitDeadline(f._open.name, input.value || null));
          legend.appendChild(set);
          if (dl) {
            const clear = el("button", "chip", "Clear");
            clear.appendChild(el("span", "k", "c"));
            clear.addEventListener("click", () => submitDeadline(f._open.name, null));
            legend.appendChild(clear);
          }
          const cancel = el("button", "chip", "Cancel");
          cancel.appendChild(el("span", "k", "esc"));
          cancel.addEventListener("click", closePrompt);
          legendLeft.appendChild(cancel);
          input.focus();
          return;
        }
        let primary;
        if (f && f._open) {
          primary = el("button", "chip primary", "Open");
          primary.appendChild(el("span", "k", "enter"));
          primary.addEventListener("click", () => renderDrill(f._open));
        } else if (f && f._item) {
          const it = f._item;
          const examPrimary = it.state === "examdue"; // a drilled deck's main action is its exam
          primary = el("button", "chip primary", examPrimary ? "Take exam" : "Learn");
          // A plain Learn subtly names the depth it'll resume at (the deck's own
          // remembered last depth); an exam-due deck's primary names its exam instead.
          // A trace has no depth (it's walked — depths don't apply), so it never gets a tag.
          if (!examPrimary && it.last_depth && !it.is_trace) primary.appendChild(el("span", "depth-tag", " ·" + it.last_depth));
          // Learn (facts → review, trace → walk) is Enter; an exam-due deck's
          // primary is its exam (also enter, or 🔒 when that exam is locked).
          primary.appendChild(el("span", "k", examPrimary && !it.examable ? "\u{1F512}" : "enter"));
          primary.disabled = !canStartPrimary(it, f._gated);
          primary.addEventListener("click", () => launch(it, f._wsName, f._gated));
        } else {
          primary = el("button", "chip primary", "Learn");
          primary.appendChild(el("span", "k", "enter"));
          primary.disabled = true;
        }
        legend.appendChild(primary);
        // The depth split: a small ▾ beside a plain Learn opens the depth menu
        // above (Recognize/Recall/Reconstruct); see `hasDepthSplit` for which
        // rows carry it.
        if (hasDepthSplit(f)) {
          const it = f._item;
          const lv = el("button", "chip split", "Depth…");
          lv.title = "choose a depth";
          lv.appendChild(el("span", "k", label(keys.depth)));
          lv.disabled = !canStart(it, f._gated);
          // Keep focus on the deck row (see the depth menu above): the click
          // rebuilds the footer, which would otherwise strand focus on <body>.
          lv.addEventListener("mousedown", e => e.preventDefault());
          lv.addEventListener("click", () => { depthMenuOpen = true; cramOn = false; syncPrimary(); });
          legend.appendChild(lv);
        }
        // A read-only Browse of the focused deck (key b).
        if (f && f._item) {
          const br = el("button", "chip", "Browse");
          br.appendChild(el("span", "k", "b"));
          br.addEventListener("click", () => browse(f._item, f._wsName));
          legend.appendChild(br);
        }
        // Augment the focused deck, workspace, or folder (key a): add / remove
        // AI augmentations. A workspace/folder opens the same screen over the
        // union of its members' cards (plus the icon target).
        if (f && (f._item || f._open)) {
          const ag = el("button", "chip", "Augment");
          ag.appendChild(el("span", "k", "a"));
          ag.addEventListener("click", () => {
            lastWorkspace = f._wsName || null;
            openAugment(f._item ? f._item.name : f._open.name);
          });
          legend.appendChild(ag);
        }
        // "Ready by…" (key d): opens the inline date prompt above to set, move,
        // or clear a workspace's personal deadline. A real workspace only — a
        // folder has no deadline concept (see catalog.rs).
        if (f && f._open && f._open.state === "workspace") {
          const rb = el("button", "chip", "Ready by…");
          rb.appendChild(el("span", "k", "d"));
          rb.addEventListener("click", () => { deadlinePromptOpen = true; syncPrimary(); });
          legend.appendChild(rb);
        }
        // Back is the header ← nav button (and Esc/Backspace); no footer chip.
        // "Take exam" sits to the RIGHT of Back for any deck that HAS an exam but
        // isn't already exam-due (where the primary is the exam): enabled to test
        // out early, or disabled with a 🔒 key hint when its exam is locked. A trace
        // always shows it — its primary is the Walk, so this is the only way to reach
        // its compression exam (whatever its drill state).
        if (f && f._item && f._item.has_exam && (f._item.is_trace || f._item.state !== "examdue")) {
          const it = f._item;
          const ex = el("button", "chip", "Take exam");
          ex.appendChild(el("span", "k", it.examable ? "x" : "\u{1F512}"));
          ex.disabled = !it.examable;
          if (it.examable) ex.addEventListener("click", () => { lastWorkspace = f._wsName || null; startExam(it.name); });
          legend.appendChild(ex);
        }
      }

      // Closes the focus drawer and forgets its selection: animate its height to 0,
      // then remove it (so the rows below glide up rather than snapping). A second
      // close never stacks a closer — the previous one is dropped at once.
      function clearDrawer() {
        drawerCycle = null;
        if (closingEl) { closingEl.remove(); closingEl = null; }
        if (drawerEl) {
          const wrap = drawerEl; drawerEl = null;
          closingEl = wrap;
          wrap.style.pointerEvents = "none";
          const done = () => { if (closingEl === wrap) closingEl = null; wrap.remove(); };
          if (wrap.animate) {
            const cur = wrap.offsetHeight;        // current height (mid-open if interrupted)
            if (wrap._anim) wrap._anim.cancel();  // stop an in-flight open
            wrap.style.height = cur + "px";       // pin so the cancel can't flash to full
            const a = wrap.animate(
              [{ height: cur + "px" }, { height: "0px" }],
              { duration: drawerDurationMs, easing: drawerEasing, fill: "forwards" }
            );
            a.onfinish = done;
          } else {
            done();
          }
        }
        drawerSel = { deck: null, topology: null, region: null };
      }

      // Builds the inline drawer for the focused deck once its topologies are known:
      // a topology picker (only when there's more than one) over a clickable region
      // heatmap ("Whole deck" first), with a due/new count at the right end that
      // follows the selection. Selecting a region scopes the launch to it. The
      // wrapper animates its height open.
      function renderDrawer(row, data) {
        const topologies = data.topologies || [];
        const heatmap = data.heatmap || [];
        const preamble = data.preamble || "";
        // Nothing worth showing → no drawer.
        if (!preamble && !heatmap.length && !topologies.length) return;
        drawerSel = { deck: row._item.name, topology: topologies[0] ? topologies[0].name : null, region: null };
        drawerCycle = null;
        const wrap = el("div", "drawer-wrap");
        const box = el("div", "drawer");

        // A size-first progress funnel pinned top-right (informative, not shocking
        // like a due backlog). The counts nest lib-side (retired ⊆ learned ⊆ seen ⊆
        // total); each component after the total is hidden while zero, so a fresh
        // deck reads as a plain "N cards".
        const total = data.total || 0;
        if (total > 0) {
          const parts = [total === 1 ? "1 card" : total + " cards"];
          if (data.seen) parts.push(data.seen + " seen");
          if (data.graduated) parts.push(data.graduated + " learned");
          if (data.retired) parts.push(data.retired + " retired");
          const top = el("div", "drawer-top");
          top.appendChild(el("span", "drawer-size", parts.join(" · ")));
          box.appendChild(top);
        }

        if (preamble) box.appendChild(el("div", "drawer-preamble", preamble));

        const regions = el("div", "drawer-regions");

        if (topologies.length) {
          const topoOf = () => topologies.find(t => t.name === drawerSel.topology) || topologies[0];
          const paint = () => {
            const topo = topoOf();
            regions.innerHTML = "";
            // mousedown preventDefault keeps focus on the deck row, so Enter/b and
            // ← / → keep working after a region is picked by mouse.
            const all = el("div", "drawer-region all" + (drawerSel.region ? "" : " sel"));
            all.appendChild(el("div", "crumb-name", "Whole deck"));
            all.addEventListener("mousedown", e => e.preventDefault());
            all.addEventListener("click", () => { drawerSel.region = null; paint(); });
            regions.appendChild(all);
            for (const reg of topo.regions) {
              const cell = el("div", "drawer-region" + (drawerSel.region === reg.name ? " sel" : ""));
              cell.appendChild(el("div", "crumb-name", reg.name));
              const bar = el("div", "crumb-bar");
              for (const s of reg.cells || []) {
                const c = el("span", "crumb-cell");
                paintHeatCell(c, s);
                bar.appendChild(c);
              }
              cell.appendChild(bar);
              cell.addEventListener("mousedown", e => e.preventDefault());
              cell.addEventListener("click", () => { drawerSel.region = reg.name; paint(); });
              regions.appendChild(cell);
            }
          };
          // Move the selection left/right through [Whole deck, …regions], wrapping.
          drawerCycle = (dir) => {
            const names = [null, ...topoOf().regions.map(r => r.name)];
            const i = Math.max(0, names.indexOf(drawerSel.region));
            drawerSel.region = names[(i + dir + names.length) % names.length];
            paint();
          };
          if (topologies.length > 1) {
            const head = el("div", "drawer-head");
            head.appendChild(el("span", "drawer-label", "Order"));
            const sel = el("select", "drawer-topo");
            for (const t of topologies) {
              const o = el("option", "", t.principle ? `${t.name} · ${t.principle}` : t.name);
              o.value = t.name;
              sel.appendChild(o);
            }
            sel.value = drawerSel.topology;
            sel.addEventListener("change", () => { drawerSel.topology = sel.value; drawerSel.region = null; paint(); });
            head.appendChild(sel);
            box.appendChild(head);
          }
          paint();
        } else if (heatmap.length) {
          // No topology: a single full-width whole-deck heatmap (not a drill target).
          const flat = el("div", "drawer-flat");
          flat.appendChild(el("div", "crumb-name", "Whole deck"));
          const bar = el("div", "crumb-bar");
          for (const s of heatmap) {
            const c = el("span", "crumb-cell");
            paintHeatCell(c, s);
            bar.appendChild(c);
          }
          flat.appendChild(bar);
          regions.appendChild(flat);
        }

        if (regions.childNodes.length) {
          const body = el("div", "drawer-body");
          body.appendChild(regions);
          box.appendChild(body);
        }
        wrap.appendChild(box);
        drawerEl = wrap;
        row.after(wrap);
        // The wrap defaults to its natural (auto) height — visible even if animation
        // is skipped. Animate its height 0 → natural with the Web Animations API
        // (which animates the property directly, no transition-trigger timing to get
        // wrong); the base stays `auto`, so it sits at the content height afterward.
        const h = wrap.offsetHeight;
        // Scrolled BEFORE the animation starts, while the wrap still stands at its
        // natural height. The drawer is fetched after the jump, so under the last
        // row it lands off-screen; scrolling once the animation has squashed it to
        // 0px reveals nothing, and scrolling after the animation lands as a late
        // jump whenever the move is long enough to actually scroll.
        if (wrap.scrollIntoView) wrap.scrollIntoView({ block: "nearest" });
        if (h && wrap.animate) {
          wrap._anim = wrap.animate(
            [{ height: "0px" }, { height: h + "px" }],
            { duration: drawerDurationMs, easing: drawerEasing }
          );
        }
      }

      // Opens/updates the drawer for the newly focused deck. Cached payloads render
      // immediately; a fresh fetch renders only if that row is still focused.
      function syncDrawer(row) {
        if (!row || !row._item) { clearDrawer(); return; }
        const name = row._item.name;
        if (drawerSel.deck === name && drawerEl) return; // already open for this deck
        clearDrawer();
        drawerSel = { deck: name, topology: null, region: null };
        const cached = topoCache[name];
        if (cached) { renderDrawer(row, cached); return; }
        api("/api/deck-drawer", post({ deck: name })).then(d => {
          const data = d || { preamble: null, heatmap: [], topologies: [], total: 0, seen: 0, graduated: 0, retired: 0 };
          topoCache[name] = data;
          if (focusedEl === row) renderDrawer(row, data);
        });
      }

      wrap.addEventListener("focusin", (e) => {
        // Focus moving into the open drawer (its dropdown or a region cell) keeps the
        // deck focused — don't treat it as leaving the row or rebuild the drawer.
        if (e.target.closest && e.target.closest(".drawer")) return;
        const row = e.target.closest ? e.target.closest(".deckrow") : null;
        if (row !== focusedEl) depthMenuOpen = false; // the menu belongs to the row it was opened on
        focusedEl = row;
        syncPrimary();
        syncDrawer(row);
      });

      // A click on empty picker space — anywhere in the stage, not just inside the
      // list, and not on a row/chip/input/drawer — must not drop keyboard focus to
      // <body>, where the row-nav keys can't reach. Keep focus on the current (or
      // first) row. Bound to the whole stage (the list is centered inside it, so the
      // margins around it count too), de-duped across renders, and inert once the
      // picker is replaced by another view.
      if (stage._refocus) stage.removeEventListener("mousedown", stage._refocus);
      stage._refocus = (e) => {
        if (!stage.contains(wrap)) return; // picker no longer showing
        if (e.target.closest(".deckrow, button, input, .drawer")) return;
        e.preventDefault(); // don't blur the row we're keeping focus on
        const v = visibleRows();
        const row = (focusedEl && v.includes(focusedEl)) ? focusedEl : v[0];
        if (row) row.focus();
      };
      stage.addEventListener("mousedown", stage._refocus);

      // The handler above only covers clicks that land inside the stage. A click
      // anywhere else still strands focus on <body>, where the row-nav keys are
      // silently dead. Catch it after the fact instead of preventing the click,
      // so selecting text elsewhere still works.
      if (stage._refocusOut) doc.removeEventListener("focusout", stage._refocusOut);
      stage._refocusOut = () => {
        if (!stage.contains(wrap)) return; // picker no longer showing
        timers.setTimeout(() => {
          if (!stage.contains(wrap)) return;
          const active = doc.activeElement;
          if (active && active !== doc.body) return; // focus landed somewhere real
          const v = visibleRows();
          const row = (focusedEl && v.includes(focusedEl)) ? focusedEl : v[0];
          if (row) row.focus();
        }, 0);
      };
      doc.addEventListener("focusout", stage._refocusOut);

      // No filter → show each row's default set (Recent hides finished/locked and
      // non-recent decks); with a filter → search every row by label. Empty
      // section headers hide themselves, and the tree flattens.
      function applyFilter() {
        const q = filter.value.trim().toLowerCase();
        lists.classList.toggle("filtering", !!q);
        for (const sec of sectionEls) {
          let shown = 0;
          for (const r of sec.rows) {
            const show = q ? r._search.includes(q) : (r._default !== false);
            r.style.display = show ? "" : "none";
            if (show) shown++;
          }
          if (sec.header) sec.header.style.display = shown ? "" : "none";
        }
        emptyHint.style.display = (q && !visibleRows().length) ? "" : "none";
      }

      filter.oninput = applyFilter;
      filter.onkeydown = (e) => {
        const v = visibleRows();
        if (e.key === "ArrowDown") { e.preventDefault(); if (v.length) v[0].focus(); }
        else if (e.key === "Enter") { e.stopPropagation(); e.preventDefault(); if (v.length) v[0].focus(); } // focus the first match, don't launch it
        else if (e.key === "Escape") { e.stopPropagation(); e.preventDefault(); if (v.length) v[0].focus(); else filter.blur(); }
        else if (e.key === "Backspace") { e.stopPropagation(); } // edit the filter, don't go back
      };

      lists.addEventListener("keydown", (e) => {
        // Keys inside the open drawer (its native topology picker) belong to it —
        // don't hijack them for row navigation.
        if (e.target.closest(".drawer")) return;
        // While the depth menu is open it owns the keys: the per-depth bindings
        // (1/2/3 by default, matching the chips' hints) start that depth — inert
        // when that chip is disabled — Esc or the depth key again closes it back
        // to the row's own chips, and Enter still reaches the global handler
        // below, which clicks the highlighted depth. Every other row-nav key is
        // inert until the menu closes.
        if (depthMenuOpen) {
          const f = focusedEl;
          const d = DEPTHS.find(l => hit(e, keys[l]));
          if (d && f && f._item && canDoDepth(f._item, d) && (cramOn || canStartAt(f._item, f._gated, d))) {
            e.preventDefault(); e.stopPropagation();
            depthMenuOpen = false;
            launchDepth(f._item, f._wsName, f._gated, d);
          } else if (hit(e, keys.cram)) {
            e.preventDefault(); e.stopPropagation(); cramOn = !cramOn; syncPrimary();
          } else if (e.key === "Escape" || hit(e, keys.depth)) {
            e.preventDefault(); e.stopPropagation(); depthMenuOpen = false; syncPrimary();
          }
          return;
        }
        const v = visibleRows();
        const cur = e.target.closest(".deckrow");
        const idx = cur ? v.indexOf(cur) : -1;
        // Up/down move between decks. Left/right step the drawer's region selection
        // when it's open (the drawer owns h/l/←/→); with no drawer, right enters a
        // workspace and left is inert (returning is Esc/Backspace). The primary
        // action — Learn a deck / Open a workspace / Take exam — is Enter, not l.
        // g/G/Home/End jump to ends; / or Ctrl-F focus the filter; b browses;
        // a augments; d opens "Ready by…" (a real workspace only); the depth
        // key (v) opens the depth menu (same gate as its ▾ chip, and only when
        // that chip is enabled); m opens Mastered.
        if (e.key === "ArrowDown" || hit(e, keys.down)) { e.preventDefault(); if (idx < v.length - 1) v[idx + 1].focus(); }
        else if (e.key === "ArrowUp" || hit(e, keys.up)) { e.preventDefault(); if (idx > 0) v[idx - 1].focus(); } // stays on the first row; the filter is only reachable via / or Ctrl-F
        else if (e.key === "g" || e.key === "Home") { e.preventDefault(); if (v.length) v[0].focus(); }
        else if (e.key === "G" || e.key === "End") { e.preventDefault(); if (v.length) v[v.length - 1].focus(); }
        else if (hit(e, keys.filter)) { e.preventDefault(); filter.focus(); }
        else if (e.key === "ArrowRight" || hit(e, keys.open)) { e.preventDefault(); if (drawerCycle) drawerCycle(1); else if (cur && cur._open) renderDrill(cur._open); } // a deck is launched with Enter (the primary chip), not l/→
        else if (e.key === "ArrowLeft" || hit(e, keys.back)) { e.preventDefault(); if (drawerCycle) drawerCycle(-1); } // back-out is Esc/Backspace only; ←/h just steps the drawer
        else if (e.key === "b" && cur && cur._item) { e.preventDefault(); browse(cur._item, cur._wsName); }
        else if (e.key === "a" && cur && (cur._item || cur._open)) { e.preventDefault(); lastWorkspace = cur._wsName || null; openAugment(cur._item ? cur._item.name : cur._open.name); }
        else if (e.key === "d" && cur && cur._open && cur._open.state === "workspace") { e.preventDefault(); deadlinePromptOpen = true; syncPrimary(); }
        else if (hit(e, keys.depth) && hasDepthSplit(cur) && canStart(cur._item, cur._gated)) { e.preventDefault(); depthMenuOpen = true; cramOn = false; syncPrimary(); }
        else if (e.key === "r") { e.preventDefault(); refresh(); } // re-scan the decks (also the ⟳ nav button)
        else if (opts.allowMastered && hit(e, keys.mastered)) { e.preventDefault(); renderMastered(); }
        else if (e.key === "x" && cur && cur._item && cur._item.examable) {
          e.preventDefault(); lastWorkspace = cur._wsName || null; startExam(cur._item.name);
        }
      });

      applyFilter();
      syncPrimary();
      const rows = visibleRows();
      // Re-land on the deck just launched (review/browse/exam/walk), so the cursor
      // doesn't jump while the user was away; otherwise focus the first row. The
      // marker is one-shot — cleared once consumed, so later re-renders (filtering)
      // behave normally.
      const want = sessionStorage.getItem("alix.lastDeck");
      sessionStorage.removeItem("alix.lastDeck");
      const target = want && rows.find(r => (r._item && r._item.name === want) || (r._open && r._open.name === want));
      if (target) target.focus(); else if (rows[0]) rows[0].focus(); else filter.focus();
    }

    function renderTop() {
      const sections = [];
      if (workspaces.length) sections.push({ title: "Workspaces", rows: workspaces.map(openRow) });
      if (recent.length) sections.push({
        title: "Recent",
        // Recent hides finished/locked and non-recent decks until you filter.
        rows: recent.map(d => deckRow(d, true, null, d.recent && d.state !== "finished" && !d.locked)),
      });
      if (folders.length) sections.push({ title: "Folders", rows: folders.map(openRow) });
      renderList({
        headerFilter: true,
        filterPlaceholder: "Search  ·  /",
        sections, back: null, allowMastered: true,
      });
    }

    // Drilled into a workspace/folder: its members as an unlock dependency tree.
    // Esc/h returns to the top list (forgetting it, so a later session lands top).
    function renderDrill(grp) {
      // Backing out re-lands the top list on the workspace/folder we came from,
      // reusing the one-shot re-land marker that a launched deck sets.
      const back = () => { lastWorkspace = null; sessionStorage.setItem("alix.lastDeck", grp.name); renderTop(); };
      renderList({
        headerFilter: true,
        filterPlaceholder: "Search  ·  /",
        lede: grp.label || grp.name,
        ledeDesc: grp.description || null,
        deadline: grp.deadline || null,
        sections: [{ title: null, rows: grp.members.map(m => deckRow(m, true, grp.name, true, true)) }],
        back, allowMastered: false,
      });
    }

    // The Mastered window: every exam-passed deck, ungated so it can be reopened.
    // A flat list — drop the tree guides a workspace member would otherwise carry.
    function renderMastered() {
      renderList({
        headerFilter: true,
        filterPlaceholder: "Search  ·  /",
        lede: "mastered \u{1F389} (reopen a deck to cram or re-examine)",
        sections: [{ title: null, rows: mastered.map(d => deckRow({ ...d, tree: "" }, false, null, true, false)) }],
        back: renderTop, allowMastered: false,
      });
    }

    // Returning from a session launched inside a workspace/folder re-opens it.
    const reopen = lastWorkspace && workspaces.concat(folders).find(g => g.name === lastWorkspace);
    if (reopen) renderDrill(reopen); else renderTop();
  }

  function handleKey(event) {
    if (!currentState() || currentState().phase !== "select") return false;
    if (event.target.closest && event.target.closest(".drawer")) return true;
    if (event.key === "Enter") {
      const primary = legend.querySelector(".chip.primary");
      if (primary && !primary.disabled) {
        event.preventDefault();
        primary.click();
      }
    } else if ((event.key === "Escape" || event.key === "Backspace") && selectEsc) {
      event.preventDefault();
      selectEsc();
    }
    return true;
  }

  return {
    focusedRowName,
    handleKey,
    rememberLaunch,
    render,
    refresh,
    select,
    setKeys,
  };
}
