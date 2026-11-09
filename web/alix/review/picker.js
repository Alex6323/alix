export function deckMetaClass(it) {
  return "meta state-" + it.state + (it.exam_changed ? " exam-changed" : "");
}

export function createPicker({
  api,
  post,
  sessionStorage,
  currentState,
  isBrowsing,
  examIsOpen,
  augmentIsOpen,
  traceIsOpen,
  tutorIsOpen,
  applyStudy,
  openTraceSession,
  openBrowse,
  openWalk,
  openTutorial,
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
  let skipIntroOn = false;
  let trackOn = null; // the depth menu's confidence toggle: null until flipped, so a launch keeps what is stored
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
    confidence: [{ k: "f", ctrl: false }],
    skip_introduction: [{ k: "s", ctrl: false }],
  };

  function setKeys(next) {
    keys = Object.assign(keys, next || {});
  }

  function rememberLaunch(name) {
    if (name) sessionStorage.setItem("alix.lastDeck", name);
    sessionStorage.setItem("alix.lastFilter", barFilter.value);
  }

  function isTraceSession(next) {
    return !!next && next.kind === "trace";
  }

  function select(name, topology, region, depth, cram, trackConfidence, skipIntroduction) {
    rememberLaunch(name);
    const body = {
      deck: name,
      topology: topology || null,
      region: region || null,
      depth: depth || null,
      cram: !!cram,
    };
    if (skipIntroduction) body.skip_introduction = true;
    if (typeof trackConfidence === "boolean") body.track_confidence = trackConfidence;
    return api("/api/select", post(body)).then((next) => {
      if (isTraceSession(next)) openTraceSession(next);
      else applyStudy(next);
      return next;
    }).catch(() => notice("could not start the session: the server log has details"));
  }

  function browse(item, workspaceName) {
    if (item.state === "error") return; // the server refuses: progress unreadable
    rememberLaunch(item.name);
    lastWorkspace = workspaceName || null;
    return openBrowse(item);
  }

  let preparing = {};
  let appliedSequence = 0;
  let preparingPoll = null;
  let preparingNotices = [];
  let resyncLegend = null;

  function showPreparingNotices() {
    if (!preparingNotices.length || !idleInSelect()) return;
    notice(preparingNotices.join(" · "));
    preparingNotices = [];
  }

  function applyPreparing(reply) {
    if (!reply || reply.sequence <= appliedSequence) return;
    appliedSequence = reply.sequence;
    const next = {};
    for (const job of reply.jobs) next[job.deck] = job;
    let settled = false;
    for (const [deck, job] of Object.entries(next)) {
      const was = preparing[deck];
      if (!was || was.state !== "generating" || job.state === "generating") continue;
      settled = true;
      preparingNotices.push(job.state === "ready"
        ? `exam ready: ${deck}`
        : `writing the exam for ${deck} failed: ${job.error || "the server log has details"}`);
    }
    preparing = next;
    const running = Object.values(next).some(job => job.state === "generating");
    if (running && !preparingPoll) preparingPoll = timers.setInterval(refreshPreparing, 5000);
    if (!running && preparingPoll) {
      timers.clearInterval(preparingPoll);
      preparingPoll = null;
    }
    if (!idleInSelect()) return;
    if (settled) render();
    else if (resyncLegend) resyncLegend();
  }

  function refreshPreparing() {
    return api("/api/exam/preparing").then(applyPreparing).catch(() => {});
  }

  function generateExam(name, needsWorkOnly) {
    rememberLaunch(name);
    return api("/api/exam/generate", post({ deck: name, needs_work_only: needsWorkOnly }))
      .then(applyPreparing)
      .catch(() => notice("could not start writing the exam: the server log has details"));
  }

  function stopExam(name) {
    return api("/api/exam/generate/cancel", post({ deck: name }))
      .then(applyPreparing)
      .catch(() => notice("could not stop writing the exam: the server log has details"));
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

  function paintHeatCell(cell, { tier, locked }) {
    cell.classList.add(tier === "unseen" ? "empty" : tier);
    if (locked) cell.classList.add("locked");
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
    currentState() && currentState().phase === "select" && !isBrowsing() && !examIsOpen() && !augmentIsOpen() && !traceIsOpen() && !tutorIsOpen();
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

  // The deck-selection screen, mirroring the terminal picker. Three groups:
  // Workspaces (each with its last-progress time), loose Decks, and
  // Folders and single-launch: click a deck to start its trace or review, or
  // open a workspace/folder to drill
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
    resyncLegend = null;
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
    showPreparingNotices();
    refreshPreparing();
    const workspaces = data.workspaces || [];
    const recent = data.recent || [];
    const folders = data.folders || [];
    if (!workspaces.length && !recent.length && !folders.length) {
      const wrap = el("div", "select");
      wrap.appendChild(el("div", "lede", "choose decks to study"));
      wrap.appendChild(el("div", "msg", "No decks found. Add .md decks to your decks folder."));
      const tutorial = el("button", "chip tutorial-start", "Tutorial");
      tutorial.addEventListener("click", () => {
        openTutorial().catch(() => notice("could not open the tutorial: the server log has details"));
      });
      wrap.appendChild(tutorial);
      stage.appendChild(wrap);
      return;
    }

    // Every mastered (exam-passed) deck across the catalog, for the m window.
    const mastered = recent.filter(d => d.mastered);
    for (const g of workspaces.concat(folders)) for (const m of g.members) if (m.mastered) mastered.push(m);

    // Can a row be started now, at *any* depth? Drilling is never gated by the
    // prerequisite lock (only the exam is - an exam-locked deck's `examable` is
    // false), so a gated view refuses only a deck with nothing due at any depth.
    // `reviewable` already folds in trace handling alongside the per-depth
    // due-ness, so this reads as "any depth or trace is startable". The
    // Mastered window is
    // ungated — a finished deck can be reopened to cram or re-examine.
    // An "error" row (unreadable progress document) is never startable, even
    // ungated: the server refuses the select, so offering it is a dead click.
    const canStart = (it, gated) => it.state !== "error" && (it.reviewable || !gated);

    // Maps a depth name to its own due-ness field (`picker::DeckStatus`'s
    // per-depth split), so each depth chip — and the plain Learn button, which
    // targets the deck's own last-used depth — gates on its own honest signal
    // rather than "any depth" (recall-settled must not enable a Recall chip
    // just because Reconstruct is due).
    const DEPTHS = ["recognize", "recall", "reconstruct"]; // menu/keys order — 1/2/3
    const DEPTH_FIELD = { recognize: "reviewable_recognize", recall: "reviewable_recall", reconstruct: "reviewable_reconstruct" };
    const canStartAt = (it, gated, depth) =>
      it.state !== "error" && (it[DEPTH_FIELD[depth]] || !gated);
    // The depth menu opens for a CRAM even when no depth is due: cram serves
    // cards that aren't due, and a deck whose every card sits in the
    // introduction cooldown is exactly when it is wanted. The menu itself
    // then offers only the cram tick-box until it is on (each depth chip
    // keeps its own honest gate), and the plain Learn button stays dead.
    const canWalk = (it) => it.state !== "error" && !it.is_trace;
    const canOpenDepths = (it, gated) =>
      canStart(it, gated) || (it.state !== "error" && !!it.crammable) || canWalk(it);
    // Recognize is pick-only: it can only run on a deck with cached choice
    // distractors (`can_recognize`) — an un-augmented deck greys it out even under
    // cram (which re-serves recognized cards). Recall/Reconstruct are never gated
    // on augmentation.
    const canDoDepth = (it, depth) => depth !== "recognize" || !!it.can_recognize;

    // Rows that carry the depth split (the Learn ▾ chip and its v key): a deck
    // with a remembered depth that is not a workspace, folder, or trace.
    const hasDepthSplit = (row) => !!(row && row._item && !row._open
      && row._item.last_launch && !row._item.is_trace);

    // The plain Learn/primary button's gate: a trace always traces; a fact deck
    // reviews at its own last-used depth or resumes its last walk.
    const walksFirst = (it) => it.last_launch === "walk" && !it.is_trace;
    const canStartPrimary = (it, gated) =>
      it.is_trace ? canStart(it, gated)
        : walksFirst(it) ? canWalk(it)
        : (canDoDepth(it, it.last_launch) && canStartAt(it, gated, it.last_launch));

    // Launch one deck/member for a drill. Launching inside a workspace remembers
    // it so leaving the session returns here.
    function launch(it, wsName, gated) {
      if (!canStartPrimary(it, gated)) return;
      if (walksFirst(it)) { launchWalk(it, wsName); return; }
      lastWorkspace = wsName || null;
      // Apply the focus drawer's topology/region pick, but only when it belongs
      // to the deck being launched (the drawer follows the focused row).
      const sel = drawerSel.deck === it.name ? drawerSel : {};
      select(it.name, sel.topology, sel.region);
    }

    // Launch at an explicit depth (from the split Learn button's ▾ menu), same
    // drawer-scope rules as `launch`, but never routes to the exam — picking a
    // depth always means "review", gated on that depth's own due-ness.
    function launchDepth(it, wsName, gated, depth) {
      if (!canDoDepth(it, depth)) return;
      if (!cramOn && !canStartAt(it, gated, depth)) return;
      lastWorkspace = wsName || null;
      const sel = drawerSel.deck === it.name ? drawerSel : {};
      select(it.name, sel.topology, sel.region, depth, cramOn, trackOn, skipIntroOn);
    }

    function launchWalk(it, wsName) {
      if (!canWalk(it)) return;
      lastWorkspace = wsName || null;
      rememberLaunch(it.name);
      openWalk(it.name).catch(() => notice("could not start the walk: the server log has details"));
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
      if (it.meta) row.appendChild(el("span", deckMetaClass(it), it.meta));
      // 🕒 nothing due — at the line end with the status, so the left gutter stays
      // tree + title. (A finished deck shows its 🎉 in the badge instead.)
      if (gated && !it.reviewable && it.state !== "finished") row.appendChild(el("span", "glyph", "\u{1F552}"));
      // Click selects (focuses) the deck, opening its focus drawer, rather than
      // launching outright; Review / Enter then launches.
      row.addEventListener("click", () => row.focus());
      row._item = it; row._gated = gated; row._wsName = wsName;
      row._examNeedsWorkOnly = true;
      row._search = (it.label || it.name).toLowerCase();
      row._default = dflt !== false;
      return row;
    }

    // Renders a grouped list with a filter, focus-driven primary button, and
    // keyboard navigation. `groups` is [{ title, rows }] (title null = no
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
        filter.value = sessionStorage.getItem("alix.lastFilter") || "";
        sessionStorage.removeItem("alix.lastFilter");
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
      filter.placeholder = `Search  ·  ${label(keys.filter)}`;

      const lists = el("div", "lists");
      const groupEls = [];
      for (const sec of opts.groups) {
        const header = sec.title ? el("div", "group-heading", sec.title) : null;
        if (header) lists.appendChild(header);
        for (const r of sec.rows) lists.appendChild(r);
        groupEls.push({ header, rows: sec.rows });
      }
      const emptyHint = el("div", "empty-hint", "No decks match.");
      lists.appendChild(emptyHint);
      wrap.appendChild(lists);
      stage.appendChild(wrap);

      const visibleRows = () =>
        Array.from(lists.querySelectorAll(".deckrow")).filter(r => r.style.display !== "none");

      function launchExam(row) {
        const it = row._item;
        lastWorkspace = row._wsName || null;
        if (it.exam_prepared) {
          startExam(it.name);
          return;
        }
        const job = preparing[it.name];
        if (job && job.state === "generating") return;
        const needsWorkOnly = it.examable && it.exam_filter !== "none" && row._examNeedsWorkOnly;
        generateExam(it.name, needsWorkOnly);
      }

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
            const b = el("button", "chip" + (d === it.last_launch ? " primary" : ""), d[0].toUpperCase() + d.slice(1));
            b.appendChild(el("span", "k", label(keys[d])));
            b.disabled = !canDoDepth(it, d) || (!cramOn && !canStartAt(it, f._gated, d));
            b.addEventListener("mousedown", e => e.preventDefault());
            b.addEventListener("click", () => { depthMenuOpen = false; launchDepth(it, f._wsName, f._gated, d); });
            legend.appendChild(b);
          }
          const walk = el("button", "chip walk" + (it.last_launch === "walk" ? " primary" : ""), "Walk");
          walk.appendChild(el("span", "k", "w"));
          walk.disabled = !canWalk(it);
          walk.addEventListener("mousedown", e => e.preventDefault());
          walk.addEventListener("click", () => { depthMenuOpen = false; launchWalk(it, f._wsName); });
          legend.appendChild(walk);
          // The cram tick-box: include cards that aren't due (a due card still
          // grades as a normal review; an early pass only re-anchors).
          const cram = el("button", "chip" + (cramOn ? " primary" : ""), (cramOn ? "☑" : "☐") + " cram");
          cram.title = "include cards that aren't due; due cards still count as normal reviews";
          cram.appendChild(el("span", "k", label(keys.cram)));
          cram.addEventListener("mousedown", e => e.preventDefault());
          cram.addEventListener("click", () => { cramOn = !cramOn; syncPrimary(); });
          legend.appendChild(cram);
          const tracking = trackOn === null ? !!it.track_confidence : trackOn;
          const track = el("button", "chip" + (tracking ? " primary" : ""), (tracking ? "☑" : "☐") + " confidence");
          track.title = "ask Sure or Unsure before each answer and record it with the review";
          track.appendChild(el("span", "k", label(keys.confidence)));
          track.addEventListener("mousedown", e => e.preventDefault());
          track.addEventListener("click", () => { trackOn = !tracking; syncPrimary(); });
          legend.appendChild(track);
          const intro = el("button", "chip" + (skipIntroOn ? " primary" : ""), (skipIntroOn ? "☑" : "☐") + " skip intro");
          intro.title = "grade new cards right away instead of introducing them first";
          intro.appendChild(el("span", "k", label(keys.skip_introduction)));
          intro.addEventListener("mousedown", e => e.preventDefault());
          intro.addEventListener("click", () => { skipIntroOn = !skipIntroOn; syncPrimary(); });
          legend.appendChild(intro);
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
          primary = el("button", "chip primary", walksFirst(it) ? "Walk" : "Learn");
          // A plain Learn subtly names the depth it will resume at. A trace has
          // no depth, so it never gets a tag.
          if (!walksFirst(it) && it.last_launch && !it.is_trace) primary.appendChild(el("span", "depth-tag", " ·" + it.last_launch));
          primary.appendChild(el("span", "k", "enter"));
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
          lv.disabled = !canOpenDepths(it, f._gated);
          // Keep focus on the deck row (see the depth menu above): the click
          // rebuilds the footer, which would otherwise strand focus on <body>.
          lv.addEventListener("mousedown", e => e.preventDefault());
          lv.addEventListener("click", () => { depthMenuOpen = true; cramOn = false; trackOn = null; skipIntroOn = false; syncPrimary(); });
          legend.appendChild(lv);
        }
        // A read-only Browse of the focused deck (key b).
        if (f && f._item) {
          const br = el("button", "chip", "Browse");
          br.appendChild(el("span", "k", "b"));
          if (f._item.state === "error") br.disabled = true;
          else br.addEventListener("click", () => browse(f._item, f._wsName));
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
        // The exam entry of a fact deck: Generate exam, then Writing exam
        // while the server writes it, then Take exam once it is prepared.
        // It is filled only when the exam is due and unlocked.
        if (f && f._item && f._item.has_exam && !f._item.is_trace) {
          const it = f._item;
          const job = preparing[it.name];
          if (!it.exam_prepared && job && job.state === "generating") {
            const minutes = Math.floor((job.elapsed || 0) / 60);
            const wait = job.estimate ? ` · ${job.estimate}` : "";
            const writing = el("button", "chip exam-writing", `Writing exam · ${minutes} min${wait}`);
            writing.disabled = true;
            legend.appendChild(writing);
            const stop = el("button", "chip", "Stop");
            stop.addEventListener("click", () => stopExam(it.name));
            legend.appendChild(stop);
            return;
          }
          const accent = it.state === "examdue" && it.examable;
          const ex = el("button", "chip" + (accent ? " exam-due" : ""), it.exam_prepared ? "Take exam" : "Generate exam");
          ex.appendChild(el("span", "k exam-key", it.examable ? "x" : "\u{1F512}"));
          ex.disabled = !it.examable;
          if (job && job.state === "failed") ex.title = job.error || "";
          if (it.examable) ex.addEventListener("click", () => launchExam(f));
          legend.appendChild(ex);
          if (it.exam_prepared) return;
          const filterable = it.examable && it.exam_filter !== "none";
          const ticked = filterable && f._examNeedsWorkOnly;
          const needsWork = el("button", "chip exam-filter", (ticked ? "☑" : "☐") + " only what needs work");
          needsWork.setAttribute("aria-pressed", String(ticked));
          needsWork.disabled = !filterable;
          needsWork.addEventListener("mousedown", e => e.preventDefault());
          if (filterable) {
            needsWork.addEventListener("click", () => {
              f._examNeedsWorkOnly = !f._examNeedsWorkOnly;
              syncPrimary();
            });
          }
          legend.appendChild(needsWork);
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
        const description = data.description || "";
        // Nothing worth showing → no drawer.
        if (!description && !heatmap.length && !topologies.length) return;
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

        if (description) box.appendChild(el("div", "drawer-description", description));

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
          // A cached drawer renders synchronously inside the focus handler, so
          // the browser's own scroll-the-focused-row runs AFTER the reveal
          // above and can push the drawer's bottom out of the list's
          // scrollport (behind the footer). Re-reveal once the height has
          // settled; by then every competing scroll has run. `onfinish` never
          // fires on a cancelled (closing) animation, and the guard skips a
          // drawer that was replaced mid-open.
          wrap._anim.onfinish = () => {
            if (drawerEl === wrap && wrap.scrollIntoView) wrap.scrollIntoView({ block: "nearest" });
          };
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
          const data = d || { description: null, heatmap: [], topologies: [], total: 0, seen: 0, graduated: 0, retired: 0 };
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

      // No filter → show each row's default set (Decks hides finished and locked
      // rows); with a filter → search every row by label. Empty
      // group headers hide themselves, and the tree flattens.
      function applyFilter() {
        const q = filter.value.trim().toLowerCase();
        lists.classList.toggle("filtering", !!q);
        for (const sec of groupEls) {
          let shown = 0;
          for (const r of sec.rows) {
            const show = q ? r._search.includes(q) : (r._default !== false);
            r.style.display = show ? "" : "none";
            if (show) shown++;
          }
          if (sec.header) sec.header.style.display = shown ? "" : "none";
        }
        const visible = visibleRows();
        if (focusedEl && !visible.includes(focusedEl)) {
          focusedEl = null;
          syncPrimary();
          syncDrawer(null);
        }
        emptyHint.style.display = (q && !visible.length) ? "" : "none";
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
          } else if (e.key === "w" && !e.ctrlKey && f && f._item && canWalk(f._item)) {
            e.preventDefault(); e.stopPropagation();
            depthMenuOpen = false;
            launchWalk(f._item, f._wsName);
          } else if (hit(e, keys.cram)) {
            e.preventDefault(); e.stopPropagation(); cramOn = !cramOn; syncPrimary();
          } else if (hit(e, keys.skip_introduction)) {
            e.preventDefault(); e.stopPropagation(); skipIntroOn = !skipIntroOn; syncPrimary();
          } else if (hit(e, keys.confidence) && f && f._item) {
            e.preventDefault(); e.stopPropagation();
            trackOn = !(trackOn === null ? !!f._item.track_confidence : trackOn);
            syncPrimary();
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
        // action, Learn a deck or Open a workspace, is Enter, not l.
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
        else if (hit(e, keys.depth) && hasDepthSplit(cur) && canOpenDepths(cur._item, cur._gated)) { e.preventDefault(); depthMenuOpen = true; cramOn = false; trackOn = null; skipIntroOn = false; syncPrimary(); }
        else if (e.key === "r") { e.preventDefault(); refresh(); } // re-scan the decks (also the ⟳ nav button)
        else if (opts.allowMastered && hit(e, keys.mastered)) { e.preventDefault(); renderMastered(); }
        else if (e.key === "x" && cur && cur._item && cur._item.examable) {
          e.preventDefault(); launchExam(cur);
        }
      });

      applyFilter();
      resyncLegend = syncPrimary;
      syncPrimary();
      const rows = visibleRows();
      // Re-land on the deck just launched (review/browse/exam/trace), so the cursor
      // doesn't jump while the user was away; otherwise focus the first row. The
      // marker is one-shot — cleared once consumed, so later re-renders (filtering)
      // behave normally.
      const want = sessionStorage.getItem("alix.lastDeck");
      sessionStorage.removeItem("alix.lastDeck");
      const target = want && rows.find(r => (r._item && r._item.name === want) || (r._open && r._open.name === want));
      if (target) target.focus(); else if (rows[0]) rows[0].focus(); else filter.focus();
    }

    function renderTop() {
      const groups = [];
      if (workspaces.length) groups.push({ title: "Workspaces", rows: workspaces.map(openRow) });
      if (recent.length) groups.push({
        title: "Decks",
        // Finished and locked decks remain reachable through search.
        rows: recent.map(d => deckRow(d, true, null, d.state !== "finished" && !d.locked)),
      });
      if (folders.length) groups.push({ title: "Folders", rows: folders.map(openRow) });
      renderList({
        headerFilter: true,
        groups, back: null, allowMastered: true,
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
        lede: grp.label || grp.name,
        ledeDesc: grp.description || null,
        deadline: grp.deadline || null,
        groups: [{ title: null, rows: grp.members.map(m => deckRow(m, true, grp.name, true, true)) }],
        back, allowMastered: false,
      });
    }

    // The Mastered window: every exam-passed deck, ungated so it can be reopened.
    // A flat list — drop the tree guides a workspace member would otherwise carry.
    function renderMastered() {
      renderList({
        headerFilter: true,
        lede: "mastered \u{1F389} (reopen a deck to cram or re-examine)",
        groups: [{ title: null, rows: mastered.map(d => deckRow({ ...d, tree: "" }, false, null, true, false)) }],
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
    keys: () => keys,
    select,
    setKeys,
  };
}
