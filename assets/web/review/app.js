"use strict";
capturePairingToken({ location, history, sessionStorage });

// Shown on a 401 (no/invalid pairing token): a full-screen prompt to paste the
// token the server printed at startup, then reload with it applied.
function showTokenGate() {
  const gate = document.getElementById("tokengate");
  if (!gate || !gate.hidden) return;
  gate.hidden = false;
  const input = document.getElementById("tokengate-input");
  const connect = () => {
    const t = input.value.trim();
    if (!t) return;
    sessionStorage.setItem("alix.token", t);
    location.reload();
  };
  document.getElementById("tokengate-connect").onclick = connect;
  input.addEventListener("keydown", (e) => { if (e.key === "Enter") connect(); });
  input.focus();
}
let clientModel = createModel(localStorage);
let state = clientModel.state;     // last StateDto
let revealed = clientModel.revealed;     // back lines shown for the current card
let citationView = clientModel.citationView; // showing the source excerpt in place of the answer
let answerConcealed = clientModel.answerConcealed; // acquire recall: the revealed answer is concealed in place for a self-test (view-only; nothing reflows)
let feedback = clientModel.feedback;  // choice result being shown ({chosen, correct, passed})
let typelineChecked = clientModel.typelineChecked; // TypeLine: TypedResult per line confirmed so far this card
let confirmingLeave = clientModel.confirmingLeave; // showing the "session not finished" leave prompt
let explainInput = clientModel.explainInput; // the typed reconstruction on an explain card (client-side)
let marks = clientModel.marks;        // per-key-point yes/no/pending for an explain checklist (client-side)
let kpCur = clientModel.keypointCursor;         // the key point the cursor is on, walked top to bottom
let drawStrokes = clientModel.drawStrokes;    // strokes on a draw card this reveal: [{tool, points:[{x,y}]}]
let drawSnapshot = clientModel.drawSnapshot; // frozen dataURL of the drawing, kept visible during self-grade
let drawTool = clientModel.drawTool;    // "pen" | "erase"
let drawCanvasEl = clientModel.drawCanvas; // the live <canvas> while drawing
// Per-device "Draw answers" preference (wired to the menu in Task 5).
let drawToggle = clientModel.drawToggle;
// The configured backend's display name ("Claude", "Copilot", …) and the
// shared "X is working…" progress line the exam and augment overlays show:
// one place, so no surface can drift back to a hardcoded backend.
function workingText(s) {
  if (s < 2) return `${tutor.backendName()} is working…`;
  if (s < 90) return `${tutor.backendName()} is working… ${s}s`;
  return `${tutor.backendName()} is working… ${Math.floor(s / 60)}m ${s % 60}s — this can take a couple of minutes`;
}
let duePoll = null;   // setInterval handle while the summary waits for a cooling card
let browsing = null;  // {cards, label, i} while the read-only browse overlay is open
let KEYS = {};        // configured review key bindings, from /api/keys
let BK = { next: [{ k: "l", ctrl: false }], prev: [{ k: "h", ctrl: false }] }; // browse pager keys, from /api/browse-keys

const apiClient = createApiClient({
  fetchImpl: window.fetch.bind(window),
  sessionStorage,
  onUnauthorized: showTokenGate,
  revision: () => state?.study_revision,
});

const stage = document.getElementById("stage");
const crumbStrip = document.getElementById("crumbStrip");
const legend = document.getElementById("legend");
const deckEl = document.getElementById("deck");
const histEl = document.getElementById("hist");
const scoreEl = document.getElementById("score");
const legendLeft = document.getElementById("legendLeft");
const legendRight = document.getElementById("legendRight");
const menuWrap = document.getElementById("menuWrap");
const barFilter = document.getElementById("barFilter");
const masteredBtn = document.getElementById("masteredBtn");
// The centered header slot shows the picker search (#barFilter) or a breadcrumb
// (#deck, during review/exam), or neither (a drill-in / mastered sub-view). The
// Mastered jump (right side) only shows on the top picker; renderList sets it.
function headerSearch() { deckEl.style.display = "none"; barFilter.style.display = ""; }
function headerBreadcrumb() { barFilter.style.display = "none"; deckEl.style.display = ""; masteredBtn.style.display = "none"; }
function headerNone() { barFilter.style.display = "none"; deckEl.style.display = "none"; masteredBtn.style.display = "none"; }
// Picker nav: ⟳ re-scans the decks (same as the `r` key) and busts the icon
// cache so regenerated emblems show without an app reload. Going back is the
// footer's Back chip (esc), consistent with the sessions' Leave chip.
const barNav = document.getElementById("barNav");
const navRefresh = document.getElementById("navRefresh");
const menu = document.getElementById("menu");

function api(path, options) { return apiClient.request(path, options, validatorFor(path)); }
function post(body) { return apiClient.postOptions(body); }
function withToken(path) { return apiClient.withToken(path); }

const picker = createPicker({
  api,
  post,
  sessionStorage,
  currentState: () => state,
  isBrowsing: () => !!browsing,
  examIsOpen: () => exam.isOpen(),
  augmentIsOpen: () => augment.isOpen(),
  walkIsOpen: () => walk.isOpen(),
  tutorIsOpen: () => tutor.isOpen(),
  applyStudy: apply,
  openWalk: (next) => walk.open(next),
  openBrowse: browseDeck,
  startExam: (deck) => exam.start(deck),
  openAugment: (deck) => augment.open(deck),
  notice,
  timers: { setTimeout: window.setTimeout.bind(window) },
  ui: {
    barFilter,
    chip,
    clearLegendSides,
    deckEl,
    document,
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
    window,
  },
});

const exam = createExam({
  api,
  post,
  rememberLaunch: picker.rememberLaunch,
  rerender: render,
  applyStudy: apply,
  updateBusy,
  workingText,
  timers: {
    setInterval: window.setInterval.bind(window),
    clearInterval: window.clearInterval.bind(window),
  },
  ui: {
    alert: window.alert.bind(window),
    chip,
    deckEl,
    document,
    el,
    headerBreadcrumb,
    histEl,
    legend,
    menuWrap,
    scoreEl,
    stage,
  },
});

const tutor = createTutor({
  api,
  post,
  rerender: render,
  updateBusy,
  timers: {
    setInterval: window.setInterval.bind(window),
    clearInterval: window.clearInterval.bind(window),
  },
  walk: {
    isOpen: () => walk.isOpen(),
    replace: (next) => walk.replace(next),
  },
  study: {
    state: () => state,
    replaceState: (next) => { state = next; },
    load,
  },
  ui: {
    appendRuns,
    appendRunsOrText,
    chip,
    contextLine,
    document,
    el,
    hit,
    keys: () => KEYS,
    label,
    legend,
    stage,
  },
});

const walk = createWalk({
  api,
  fetchApi: apiClient.fetch,
  post,
  rerender: render,
  applyStudy: apply,
  sessionStorage,
  examStart: exam.start,
  tutor: {
    isOpen: tutor.isOpen,
    open: tutor.show,
    close: tutor.close,
    render: tutor.render,
  },
  ui: {
    appendRunsOrText,
    buildCardShell,
    chip,
    deckEl,
    el,
    headerBreadcrumb,
    histEl,
    hit,
    keys: () => KEYS,
    label,
    legend,
    legendLeft,
    legendRight,
    menuWrap,
    renderSourceExcerpt,
    requestAnimationFrame: window.requestAnimationFrame.bind(window),
    scoreEl,
    setMenuContext,
    setTimeout: window.setTimeout.bind(window),
    sourceTerms,
    stage,
    updateFade,
  },
});

const augment = createAugment({
  api,
  post,
  rememberLaunch: picker.rememberLaunch,
  rerender: render,
  applyStudy: apply,
  workingText,
  backendName: tutor.backendName,
  timers: {
    setInterval: window.setInterval.bind(window),
    clearInterval: window.clearInterval.bind(window),
  },
  ui: {
    chip,
    confirm: window.confirm.bind(window),
    deckEl,
    document,
    el,
    headerBreadcrumb,
    histEl,
    legend,
    menuWrap,
    scoreEl,
    stage,
  },
});

const sheets = createSheets({
  api,
  fetchApi: apiClient.fetch,
  post,
  withToken,
  focusedRowName: picker.focusedRowName,
  notice,
  refreshPicker: picker.render,
  timers: {
    setInterval: window.setInterval.bind(window),
    clearInterval: window.clearInterval.bind(window),
  },
  ui: {
    document,
    el,
    FileReader: window.FileReader,
    Option: window.Option,
  },
});
// On load the page may be seeded in browse mode (a `alix browse --serve` launch):
// the state then carries the browse phase + cards, so open the overlay directly.
function load() { return api("/api/state").then(s => { if (s.phase === "browse") { browsing = { cards: s.cards, label: s.label, i: 0 }; state = s; render(); } else apply(s); }); }
// A rejected mutation (a stale revision after a lost reply, or a transport
// failure) refetches the state, so the next click carries a fresh echo
// instead of silently doing nothing forever.
function grade(g)  { api("/api/grade", post({ grade: g })).then(apply).catch(() => load()); }
function skip()    { api("/api/skip", post({})).then(apply).catch(() => load()); }
function acquire() { api("/api/acquire", post({})).then(apply).catch(() => load()); }
function remove()  { api("/api/remove", post({})).then(apply).catch(() => load()); }
function promote() { api("/api/promote", post({})).then(apply).catch(() => load()); }
function restart() { api("/api/restart", post({})).then(apply).catch(() => load()); }

// One heatmap cell's fill: the lib's per-card tier. untouched = neutral,
// seen = dim grey (presented, never yet correct), acquired = white (correct
// once, not graduated), learned-strong/-fading/-weak = green/yellow/red by
// current retrievability, retired = purple.
function paintHeatCell(cellEl, tier) {
  cellEl.classList.add(tier === "untouched" ? "empty" : tier);
}
// A transient notice for errors the page would otherwise swallow (error
// responses carry bare status codes) — fades after a few seconds.
function notice(msg) {
  let n = document.getElementById("notice");
  if (!n) {
    n = document.createElement("div");
    n.id = "notice";
    document.body.appendChild(n);
  }
  n.textContent = msg;
  n.classList.add("show");
  clearTimeout(n._t);
  n._t = setTimeout(() => n.classList.remove("show"), 4000);
}

// Save failures are stateful, not transient: the server keeps reporting
// `save_error` until a save succeeds, so the banner stays exactly that long.
// The raw store error rides on the tooltip.
function syncSaveAlert() {
  let a = document.getElementById("save-alert");
  const msg = state && state.save_error;
  if (!msg) { if (a) a.remove(); return; }
  if (!a) {
    a = document.createElement("div");
    a.id = "save-alert";
    document.body.appendChild(a);
  }
  a.title = msg;
  a.textContent = "Progress is not being saved. Reopen the deck; recent grades may be lost.";
}

// Browse a deck read-only: the server builds the card list and returns it; open
// the in-page browse overlay (no page nav). The picker owns the return target.
function browseDeck(it) {
  return api("/api/browse", post({ deck: it.name })).then(d => { browsing = { cards: d.cards, label: d.label, i: 0 }; render(); return d; });
}
function closeBrowse() {
  clientModel = enterPicker({ ...clientModel, state, browsing, walk: walk.data() });
  state = clientModel.state;
  browsing = clientModel.browsing;
  walk.replace(clientModel.walk);
  api("/api/deselect", post({})).then(apply);
}
function browseGo(delta) { if (!browsing) return; const n = browsing.i + delta; if (n >= 0 && n < browsing.cards.length) { browsing.i = n; render(); } }
function deselect() { confirmingLeave = false; menu.classList.remove("open"); api("/api/deselect", post({})).then(apply); }

// Returning to the picker mid-session abandons the cards still queued, so warn
// first; a finished session (or the select phase) leaves straight away.
function leaveSession() {
  if (!state || state.phase !== "review") { deselect(); return; }
  confirmingLeave = true;
  renderLeaveConfirm();
}
function cancelLeave() { confirmingLeave = false; render(); }
function renderLeaveConfirm() {
  legend.innerHTML = "";
  legend.appendChild(el("span", "leave-msg", `Session not finished — ${state.remaining} card${state.remaining === 1 ? "" : "s"} left.`));
  chip("Leave anyway", "again", deselect, "enter");
  chip("Stay", "primary", cancelLeave, "esc");
}

function apply(s) {
  clientModel = applyStudyState({
    ...clientModel,
    state,
    browsing,
    walk: walk.data(),
    revealed,
    citationView,
    answerConcealed,
    feedback,
    typelineChecked,
    confirmingLeave,
    explainInput,
    marks,
    keypointCursor: kpCur,
    drawStrokes,
    drawSnapshot,
    drawTool,
    drawCanvas: drawCanvasEl,
  }, s);
  state = clientModel.state;
  walk.replace(clientModel.walk);
  revealed = clientModel.revealed;
  citationView = clientModel.citationView;
  answerConcealed = clientModel.answerConcealed;
  feedback = clientModel.feedback;
  typelineChecked = clientModel.typelineChecked;
  confirmingLeave = clientModel.confirmingLeave;
  explainInput = clientModel.explainInput;
  marks = clientModel.marks;
  kpCur = clientModel.keypointCursor;
  drawStrokes = clientModel.drawStrokes;
  drawSnapshot = clientModel.drawSnapshot;
  drawTool = clientModel.drawTool;
  drawCanvasEl = clientModel.drawCanvas;
  render();
}
function hasKeypoints() { return isExplain() && state.keypoints && state.keypoints.length > 0; }
// A never-seen card: an attempt then reveal, acknowledged with one key (not graded).
function isAcquire() { return !!(state && state.acquire); }
// First encounter as a recognition question (strictly-augmented atomic card).
function isAcquireChoice() { return isAcquire() && !!state.choices; }
function isChoice() { return state.mode === "choice" && state.choices; }
function isInput() { return state.mode === "typing"; }
function isTypeLine() { return state.mode === "typeline"; }
function isExplain() { return state.mode === "explain"; }
// A genuine Recognize-session MC pick (never true for the acquire on-ramp,
// which shows its own recognition question regardless of depth).
function isRecognizeMc() { return !isAcquire() && isChoice(); }
// The Recognize fallback: the session is Recognize but no MC could be built
// (too few distractors) — attempt→reveal with a plain Knew-it/Not-yet call,
// not the generic three-way grade.
function isRecognizeFallback() { return !isAcquire() && state.depth === "recognize" && !state.choices; }
// Draw is effective when the card is authored draw-only OR the per-device toggle
// is on — but only for the self-graded modes L1 supports, and never while acquiring.
function effectiveDraw() {
  if (!state || !state.card) return false;
  if (state.card.context && state.card.context.length) return false; // cloze cards don't draw in L1 (a mode-less cloze resolves to flip)
  if (state.mode !== "flip" && state.mode !== "explain") return false;
  return state.input === "draw" || drawToggle;
}
// The badge names the check you're doing *right now* ("flip"/"line"/"typing"/
// "explain"/"choice") so how you interact is clear up front, prefixed with provenance
// ("new" / "remediation") when it applies. Crucially it badges the *present*
// interaction, not the scheduled one: whenever choices are on screen — a recognition
// MC on a first encounter, or choice mode — it's a pick-one, so show "choice", never
// the "flip" the card's schedule will use once acquired.
// Acquire is the exception: it's an ungraded attempt-first reveal, never a graded
// check (a recognition pick just leads to "Seen") — `state.mode` is the depth's
// check regardless, so naming it here would claim a check that isn't happening.
function modeLabel() {
  if (isAcquire()) return "new";
  const kind = state.promotable ? "remediation" : "";
  const check = state.choices ? "choice" : state.mode === "typeline" ? "typing · line" : state.mode;
  return kind ? kind + " · " + check : check;
}
// A pick's result is pure evidence (the grade is separate, via /api/grade).
// Every pick shows its feedback screen (chosen + correct options highlighted).
// The acquire on-ramp's is ungraded — any pick just leads to "Seen". A genuine
// Recognize pick pauses on Continue: "Next" plus the quiet "I guessed"
// override when correct, a plain "Continue" (grades failed) when wrong — so a
// miss shows the right answer before the card moves on, same as any other check.
function choose(i) {
  api("/api/choose", post({ index: i })).catch(() => { load(); return Promise.reject(); }).then(f => {
    feedback = f;
    // Only the answer changed. A full render() rebuilds the question too, which
    // reads as a flicker (and re-rasterises any math in it).
    fillBottom();
    renderLegend();
  });
}
function submitCheck() {
  const lines = Array.from(document.querySelectorAll("#ansRegion input.field")).map(i => i.value);
  api("/api/check", post({ lines })).catch(() => { load(); return Promise.reject(); }).then(f => { feedback = f; fillBottom(); renderLegend(); }).catch(() => load());
}
// TypeLine: one line at a time; the server derives the position-paired check
// from the card's own mode. Resubmits every line checked so far (the
// previously-graded inputs plus the new one) so the server always pairs by
// true position; the last request's response IS the full result set, so once
// it covers every back line it doubles as `feedback` for the closing
// three-way grade.
function submitTypeLine(value) {
  const lines = typelineChecked.map(r => r.input).concat([value]);
  api("/api/check", post({ lines })).then(f => {
    typelineChecked = f.results;
    if (typelineChecked.length >= backCount()) feedback = f;
    render();
  });
}
// The "Check" legend chip and the field's own Enter both submit the current
// (next-unchecked) line's typed value.
function submitCurrentTypeLine() {
  const inp = document.querySelector("#ansRegion input.field");
  submitTypeLine(inp ? inp.value : "");
}

// The drawer's open/close height animation: quick.
const DRAWER_MS = 170;
const DRAWER_EASE = "cubic-bezier(0.4, 0, 0.2, 1)";
function backCount() { return state.card ? state.card.back.length : 0; }
function fullyRevealed() { return backCount() === 0 || revealed >= backCount(); }

// Does a keydown match one of a binding list (from the config)?
function hit(e, binds) {
  return (binds || []).some(b => b.ctrl === e.ctrlKey && e.key.toLowerCase() === b.k.toLowerCase());
}
// A short label for the first binding of an action, for the chip key-hints.
function label(binds) {
  if (!binds || !binds.length) return "";
  const b = binds[0];
  const name = b.k === " " ? "space" : b.k === "Enter" ? "enter"
    : b.k === "Escape" ? "esc" : b.k === "Backspace" ? "bksp" : b.k;
  return (b.ctrl ? "ctrl-" : "") + name;
}

// While the summary is showing, a card the user missed may cool back into
// due-ness. Poll /api/state (which re-checks server-side) so it re-enters review
// on its own — no manual restart. Only re-renders when a card actually returns,
// so the summary doesn't re-animate on every tick.
function stopDuePoll() { if (duePoll) { clearInterval(duePoll); duePoll = null; } }
function startDuePoll() {
  if (duePoll) return;
  duePoll = setInterval(() => {
    api("/api/state").then(s => {
      if (s.phase === "review" && s.card) { stopDuePoll(); apply(s); }
    }).catch(() => {});
  }, 3000);
}

function render() {
  stopDuePoll();
  syncSaveAlert();
  stage.innerHTML = "";
  crumbStrip.innerHTML = "";
  legend.innerHTML = "";
  clearLegendSides();
  menu.classList.remove("open");

  if (exam.isOpen()) { exam.render(); return; }
  if (augment.isOpen()) { augment.render(); return; }
  const screen = currentScreen({ ...clientModel, state, browsing, walk: walk.data() });
  if (screen === "walk") { walk.render(); return; }
  if (screen === "browse") { renderBrowse(); return; }
  if (screen === "picker") { picker.render(); return; }

  headerBreadcrumb();
  deckEl.textContent = state.label;
  // The one header readout during review: a dim "N left" convergence token.
  // The card pile hints at it but caps at 3 layers, so 40 left and 4 left look
  // the same there; the number may honestly tick UP when a missed card cools
  // back in. Anything richer (score breakdowns, ETA) stays out: noise.
  histEl.textContent = "";
  if (state.phase === "review") histEl.appendChild(el("span", "left-token", `${state.remaining} left`));
  scoreEl.innerHTML = ""; // the per-review score readout is intentionally omitted (noise)
  menuWrap.style.display = state.phase === "done" ? "none" : "";
  setMenuContext("review");

  if (tutor.isOpen()) { tutor.render(); return; }
  if (screen === "summary") { renderSummary(); startDuePoll(); }
  else renderCard();
}

// ── ask tutor ─────────────────────────────────────────────────────────
// Available once a card is answered. Sends run the CLI on the server; we poll
// /api/ask while it's thinking. One conversation spans the session.
function isAnswered() {
  if (!state || state.phase !== "review" || !state.card) return false;
  if (feedback) return true;
  return !isChoice() && !isInput() && fullyRevealed();
}

// Loop the header logo while any backend/server call is in flight.
function updateBusy() {
  const logo = document.querySelector(".brand alix-logo");
  if (logo) logo.toggleAttribute("loop", !!(tutor.isPolling() || exam.isPolling()));
}

// Replay the header logo's reveal once, for the reload button and the `r` key.
function replayLogo() {
  const logo = document.querySelector(".brand alix-logo");
  if (logo && logo.replay) logo.replay();
}


// A CLI action while the tab is blurred — `alix receive`, `alix generate`, a
// file dropped into the decks dir — adds decks the open picker can't see. The
// catalog is read fresh from disk on every fetch, so regaining focus in the
// select phase re-scans (same as the ⟳ button). The scan is QUIET: the screen
// repaints only when the catalog actually changed, so a plain alt-tab back
// never visibly refreshes the picker. Overlays and sessions are left alone.

// Build the shared card scaffold: .stack > .card > (.region.q) + .divider +
// (.region.a) + (optional .more-hint), appended to `stage`. Returns refs to
// the empty nodes so callers can fill them without touching the DOM again.
//   cardId    — set card.id (renderCard needs "card" for setNote lookups)
//   ansId     — set a.id   (renderCard needs "ansRegion" for fillBottom)
//   withNote  — adds "withnote" class to the answer region (caps it shorter)
//   moreHint  — appends .more-hint for the overflow marker (browse omits it)
//   leftAlign — sets textAlign:"left" on q and a (walk excerpt is always left)
function buildCardShell({ pile, cardId = null, ansId = null, withNote = false, moreHint = true, leftAlign = false } = {}) {
  const stack = el("div", "stack");
  stack.dataset.pile = Math.min(3, Math.max(1, pile));
  stack.appendChild(el("div", "peek p2"));
  stack.appendChild(el("div", "peek p1"));
  const card = el("div", "card");
  if (cardId) card.id = cardId;
  const q = el("div", "region q");
  if (leftAlign) q.style.textAlign = "left";
  card.appendChild(q);
  card.appendChild(el("div", "divider"));
  const a = el("div", "region a" + (withNote ? " withnote" : ""));
  if (ansId) a.id = ansId;
  if (leftAlign) a.style.textAlign = "left";
  a.addEventListener("scroll", () => updateFade(a));
  card.appendChild(a);
  if (moreHint) {
    card.appendChild(el("div", "more-hint"));      // "more below", pinned to the answer's bottom edge
    card.appendChild(el("div", "more-hint top"));  // "more above", pinned to the answer's top edge
  }
  stack.appendChild(card);
  stage.appendChild(stack);
  return { stack, card, q, a };
}

function renderCard() {
  const c = state.card;
  const hasNote = c.note && c.note.length > 0;
  const { card, q } = buildCardShell({
    pile: state.remaining,
    cardId: "card",
    ansId: "ansRegion",
    withNote: hasNote,
  });
  // The type badge heads the card (top, centered), above the question — it names the
  // present check (modeLabel), doesn't change within a card, and survives the
  // answer region's re-renders because it lives on the card, not inside it.
  card.insertBefore(el("span", "mode-tag", modeLabel()), card.firstChild);

  // question region
  // Orientation breadcrumb — rendered into the #crumbStrip pinned just below the
  // header hairline, centered: each region is its name (bold = where you are) over a
  // thin per-card heatmap bar, every card a tier cell (see paintHeatCell), so the
  // line doubles as a progress map that greens up as a region is learned.
  if (c.crumb && c.crumb.regions.length) {
    const cr = c.crumb;
    const bc = el("div", "crumb");
    for (let i = 0; i < cr.regions.length; i++) {
      const reg = el("div", "crumb-region" + (i === cr.current ? " cur" : ""));
      reg.appendChild(el("div", "crumb-name", cr.regions[i]));
      const bar = el("div", "crumb-bar");
      for (const s of (cr.cells && cr.cells[i]) || []) {
        const cell = el("span", "crumb-cell");
        paintHeatCell(cell, s);
        bar.appendChild(cell);
      }
      reg.appendChild(bar);
      bc.appendChild(reg);
    }
    crumbStrip.appendChild(bc);
  }
  const frontNode = frontEl(c.front, c.front_runs, c.front_units);
  // On a cloze card the front is the topic and the gapped sentence below it is
  // the actual question, so the sentence leads and the topic steps back.
  if (c.context.length) frontNode.classList.add("topic");
  q.appendChild(frontNode);
  for (let i = 0; i < c.context.length; i++) {
    q.appendChild(contextLine(c.context[i], c.context_runs && c.context_runs[i]));
  }
  appendImages(q, c.images);

  // The answer region (id="ansRegion") is filled by fillBottom(); the note region
  // and its divider are added only once the note is shown (see setNote); the answer
  // is capped shorter only when the card has a note, to leave room for it.
  fillBottom();
  renderLegend();
}

// Read-only browse: step through every card in a deck — front, the revealed
// answer (with the format reshape's bullets + notes), no grading. An in-page
// overlay reached from the picker's Browse action or `alix browse --serve`;
// there is no separate /browse page.
function renderBrowse() {
  headerBreadcrumb();
  deckEl.textContent = browsing.label;
  histEl.textContent = "";
  scoreEl.innerHTML = `<span class="left">${browsing.i + 1} / ${browsing.cards.length}</span>`;
  menuWrap.style.display = "none";
  const c = browsing.cards[browsing.i];
  const hasNote = c.note && c.note.length > 0;

  const { card, q, a } = buildCardShell({
    pile: browsing.cards.length - browsing.i,
    withNote: hasNote,
    moreHint: false, // browse has no overflow marker
  });

  const frontNode = frontEl(c.front, c.front_runs, c.front_units);
  // On a cloze card the front is the topic and the gapped sentence below it is
  // the actual question, so the sentence leads and the topic steps back.
  if (c.context.length) frontNode.classList.add("topic");
  q.appendChild(frontNode);
  for (let i = 0; i < c.context.length; i++) {
    q.appendChild(contextLine(c.context[i], c.context_runs && c.context_runs[i]));
  }
  appendImages(q, c.images);

  const sec = el("div", "reveal" + (leftAlignAnswer(c) ? " list" : ""));
  if (isReshapedList(c)) appendReveal(sec, c.back, c.back_runs, true);
  else appendAnswerUnits(sec, c.back_units);
  a.appendChild(sec);
  appendImages(a, c.images_back);
  a.classList.add("has-body"); // browse always shows the full answer

  if (hasNote) {
    card.appendChild(el("div", "divider"));
    const n = el("div", "region n");
    renderNote(n, c.note);
    card.appendChild(n);
  }

  updateFade(a); // content-aware placement: center a short answer, top-align a long one

  chip("Prev", "", () => browseGo(-1), label(BK.prev)).disabled = browsing.i === 0;
  chip("Next", "primary", () => browseGo(1), label(BK.next)).disabled = browsing.i >= browsing.cards.length - 1;
  chip("Leave", "", closeBrowse, "esc");
}

// Fills the answer region for the current mode/phase, and shows the note (with
// its own divider) only when it should be visible.
function fillBottom() {
  const a = document.getElementById("ansRegion");
  if (!a) return;
  a.innerHTML = "";
  const citations = state.card.citations || [];
  const citable = citations.length > 0 && isAnswered();
  if (citable && citationView) {
    // Source view: all cited excerpts take the answer's place in authored order.
    renderSourceCitations(a, citations);
    setNote(true);
  } else if (isAcquire()) {
    if (effectiveDraw()) {
      // Attempt-first, ungraded: draw your answer, then reveal it to compare.
      if (revealed === 0) { renderDrawCanvas(a); setNote(false); }
      else {
        if (drawSnapshot) a.appendChild(frozenDrawImg(drawSnapshot)); // your attempt, kept for comparison
        fillAcquire(a); setNote(true);         // then the answer (still just "Seen")
      }
    } else if (isAcquireChoice()) {
      if (feedback) renderChoiceFeedback(a); else renderChoices(a);
      setNote(!!feedback);
    } else if (revealed > 0) {
      fillAcquire(a); setNote(true);          // recall: answer shown after reveal
    } else {
      a.appendChild(el("div", "acquire-hint", "new card — try to recall it, then reveal."));
      setNote(false);                          // recall: front only until revealed
    }
  }
  else if (feedback) { (isChoice() ? renderChoiceFeedback : renderCheckFeedback)(a); setNote(true); }
  else if (isChoice()) { renderChoices(a); setNote(false); }
  else if (isInput()) { renderInput(a); setNote(false); }
  else if (isTypeLine()) { renderTypeLine(a); setNote(false); }
  else if (effectiveDraw()) {
    if (revealed === 0) { renderDrawCanvas(a); setNote(false); }
    else {
      if (drawSnapshot) a.appendChild(frozenDrawImg(drawSnapshot)); // your drawing, for comparison
      if (isExplain()) { renderExplain(a); setNote(true); }         // key-points checklist reveal
      else { fillAnswer(a); setNote(true); }                        // flip reveal (back / img_back)
    }
  }
  else if (isExplain()) { renderExplain(a); setNote(fullyRevealed()); }
  else { fillAnswer(a); setNote(fullyRevealed()); }
  // A cited card: the whole region toggles answer ⟷ source (click it, or `s`).
  // The pill both marks the card as having a source and labels where it is.
  a.classList.toggle("citable", citable);
  a.onclick = citable ? onCiteClick : null;
  if (citable) {
    const grp = el("span", "cite-toggle");
    grp.title = citationView
      ? "show answer"
      : citations.length === 1
        ? "show source " + citations[0].locator
        : `show ${citations.length} sources`;
    grp.appendChild(el("span", "ci", citationView ? "¶" : "</>"));
    grp.appendChild(el("span", "k", "s"));
    a.appendChild(grp);
  }
  // Acquire recall: the same corner-cue mechanism as the source swap, here hiding /
  // un-hiding the revealed answer in place so you can self-test the encoding. `h` (or
  // a tap on the region) flips it both ways. Shown only once the answer is revealed
  // (nothing to hide before then), and never on a cited card — citation owns the corner.
  const hidable = isAcquire() && !effectiveDraw() && !isAcquireChoice() && citations.length === 0 && revealed > 0;
  a.classList.toggle("hidable", hidable);
  a.classList.toggle("concealed", hidable && answerConcealed);
  if (hidable) {
    a.onclick = onAcqToggleClick;
    const grp = el("span", "cite-toggle");
    grp.title = answerConcealed ? "show answer" : "hide the answer to self-test";
    grp.appendChild(el("span", "ci", answerConcealed ? "⊙" : "⊘"));
    grp.appendChild(el("span", "k", "h"));
    a.appendChild(grp);
  }
  // keep the newest line in view as a verse is revealed line by line
  if (state.mode === "line" && revealed > 0) a.scrollTop = a.scrollHeight;
  // Content-aware placement (applied by updateFade): a short answer centers below
  // the midline once there's real body content, sitting clearly separated from the
  // prompt. A line card centers too — it grows as each line reveals and re-settles,
  // which is fine; if it grows past the region it overflows into `filled` and the
  // per-line auto-scroll (above) keeps the newest line reachable. A short cited
  // source follows the same centering rule; a long one overflows into `filled`
  // and stays top-aligned and scrollable. The pre-reveal badge/hint alone isn't
  // body to center.
  a.classList.toggle("has-body", !!a.querySelector(
    ".reveal, .options, .inputs, .source-excerpt, .kp-list, .explain-answer, img.card-img, .cite-err"));
  updateFade(a);
}

// Swap the answer region between the worded answer and the cited source excerpt.
function toggleCitation() {
  if (!state || !state.card || !(state.card.citations || []).length || !isAnswered()) return;
  citationView = !citationView;
  fillBottom();
}
// Click anywhere in the answer/excerpt swaps it — but don't hijack a drag that's
// selecting text (e.g. copying a line of the excerpt).
function onCiteClick() {
  if (window.getSelection && String(window.getSelection())) return;
  toggleCitation();
}

// Show a soft fade on whichever edge of the answer region hides content,
// instead of a scrollbar.
// Count source-excerpt lines fully below the region's visible bottom, so the
// marker can say how much more there is.
function hiddenLineCount(a) {
  const lns = a.querySelectorAll(".source-line");
  if (!lns.length) return 0;
  const foldY = a.getBoundingClientRect().bottom;
  let n = 0;
  lns.forEach(ln => { if (ln.getBoundingClientRect().top >= foldY - 4) n++; });
  return n;
}

function updateFade(a) {
  if (!a) return;
  // Content-aware placement: center a short answer that fits (it settles below the
  // midline), top-align one that overflows so its top stays reachable. `has-body`
  // (set by fillBottom) gates it, so a badge-only pre-reveal region isn't centered.
  const hints = overflowHints(a);
  const hasBody = a.classList.contains("has-body");
  a.classList.toggle("balanced", hasBody && !hints.overflows);
  a.classList.toggle("filled", !hasBody || hints.overflows);
  a.classList.toggle("fade-top", hints.showTop);
  a.classList.toggle("fade-bottom", hints.showBottom);
  const parent = a.parentElement;
  if (!parent) return;
  const below = parent.querySelector(".more-hint:not(.top)");
  const above = parent.querySelector(".more-hint.top");
  // Pin the hints to the answer region's own top/bottom edges (not the card's),
  // so "more below" sits above the note's divider instead of over the note.
  const cardH = a.offsetParent ? a.offsetParent.clientHeight : parent.clientHeight;
  const aTop = a.offsetTop;
  const aBottom = a.offsetTop + a.offsetHeight;
  if (below) {
    below.style.bottom = Math.max(0, cardH - aBottom + 8) + "px";
    if (hints.showBottom) {
      const n = hiddenLineCount(a);
      below.textContent = n > 0 ? `⌵ ${n} more line${n === 1 ? "" : "s"}` : "⌵ more below";
      below.classList.add("show");
    } else below.classList.remove("show");
  }
  if (above) {
    above.style.top = (aTop + 8) + "px";
    if (hints.showTop) { above.textContent = "⌃ more above"; above.classList.add("show"); }
    else above.classList.remove("show");
  }
}

// Adds (or removes) the note region and a divider before it. Because content is
// top-aligned, adding it on reveal doesn't shift the question or answer, so it
// only needs to exist while shown — no premature empty zone or stray divider.
function setNote(show) {
  const card = document.getElementById("card");
  if (!card) return;
  let divider = document.getElementById("noteDivider");
  let n = document.getElementById("noteRegion");
  const has = state.card.note && state.card.note.length > 0;
  if (show && has) {
    if (!divider) { divider = el("div", "divider"); divider.id = "noteDivider"; card.appendChild(divider); }
    if (!n) { n = el("div", "region n"); n.id = "noteRegion"; card.appendChild(n); }
    n.innerHTML = "";
    renderNote(n, state.card.note);
  } else {
    if (divider) divider.remove();
    if (n) n.remove();
  }
}

// Inline-code runs in key points are an explicit author signal: highlight those
// exact, case-sensitive terms in the source without guessing from prose.
function sourceTerms(runGroups) {
  const terms = new Set();
  for (const runs of runGroups || []) {
    for (const run of runs || []) {
      if (run && run.code && run.text && run.text.trim()) terms.add(run.text);
    }
  }
  return Array.from(terms).sort((a, b) => b.length - a.length || a.localeCompare(b));
}

function appendSourceText(parent, text, terms) {
  let cursor = 0;
  while (cursor < text.length) {
    let nextAt = -1;
    let nextTerm = "";
    for (const term of terms || []) {
      const at = text.indexOf(term, cursor);
      if (at >= 0 && (nextAt < 0 || at < nextAt || (at === nextAt && term.length > nextTerm.length))) {
        nextAt = at;
        nextTerm = term;
      }
    }
    if (nextAt < 0) {
      parent.appendChild(document.createTextNode(text.slice(cursor)));
      return;
    }
    if (nextAt > cursor) parent.appendChild(document.createTextNode(text.slice(cursor, nextAt)));
    parent.appendChild(el("mark", "source-term", nextTerm));
    cursor = nextAt + nextTerm.length;
  }
}

// One editor-style source excerpt shared by fact citations and trace reveals.
function renderSourceExcerpt(parent, ex, terms) {
  const panel = el("div", "source-excerpt");
  panel.appendChild(el("div", "source-file", ex.path));
  const code = el("div", "source-code");
  for (const ln of ex.lines) {
    const row = el("div", "source-line");
    row.appendChild(el("span", "source-number", String(ln.n)));
    const text = el("span", "source-text");
    appendSourceText(text, ln.text, terms);
    row.appendChild(text);
    code.appendChild(row);
  }
  panel.appendChild(code);
  if (ex.truncated) panel.appendChild(el("div", "source-truncated", "… excerpt truncated"));
  parent.appendChild(panel);
}

function renderSourceCitations(parent, citations) {
  const stack = el("div", "source-stack");
  for (const citation of citations || []) {
    if (citation.excerpt) renderSourceExcerpt(stack, citation.excerpt);
    else {
      stack.appendChild(el(
        "div",
        "cite-err",
        `⚠ ${citation.locator}: ${citation.error || "source unavailable"}`,
      ));
    }
  }
  parent.appendChild(stack);
}

// Tappable multiple-choice options.
// The keyboard-focused MC option (up/down via the configured nav keys or arrows),
// starting on the first. Enter picks the focused one; number keys and click still work.
let choiceFocus = -1;
function moveChoiceFocus(delta) {
  const opts = document.querySelectorAll(".options .option");
  if (!opts.length) return;
  choiceFocus = choiceFocus < 0
    ? (delta > 0 ? 0 : opts.length - 1)
    : (choiceFocus + delta + opts.length) % opts.length;
  opts.forEach((o, i) => o.classList.toggle("focused", i === choiceFocus));
  opts[choiceFocus].scrollIntoView({ block: "nearest" });
}
function renderChoices(a) {
  choiceFocus = -1;
  const first = appendChoiceOptions(a, {
    choices: state.choices,
    choiceRuns: state.choice_runs,
    onChoose: choose,
  });
  if (first) { choiceFocus = 0; first.classList.add("focused"); }
}

// Typing: an input per answer line, submitted with Enter or the chip.
function renderInput(a) {
  const wrap = el("div", "inputs");
  state.card.back.forEach(() => {
    const inp = el("input", "field");
    inp.type = "text"; inp.autocomplete = "off"; inp.spellcheck = false;
    inp.addEventListener("keydown", e => { if (e.key === "Enter") { e.preventDefault(); submitCheck(); } });
    wrap.appendChild(inp);
  });
  a.appendChild(wrap);
  const first = wrap.querySelector("input");
  if (first) first.focus();
}

// One checked typed line: a trailing ✓ (green) or ✗ (red); a wrong line also
// shows the expected answer beneath it (a miss — typo or genuinely wrong, the
// learner decides which — isn't memorised silently). Shared by the plain typed
// check's full-card feedback and TypeLine's line-by-line history.
function checkedLine(wrap, r) {
  const line = el("div", "answer" + (r.passed ? " pass" : " miss"));
  line.appendChild(el("span", "txt", r.input || "—"));
  line.appendChild(el("span", "mark", r.passed ? "✓" : "✗"));
  wrap.appendChild(line);
  if (!r.passed) wrap.appendChild(el("div", "expected", r.expected));
}

// The typed answer after submitting: every line, checked.
function renderCheckFeedback(a) {
  const wrap = el("div", "reveal");
  feedback.results.forEach(r => checkedLine(wrap, r));
  a.appendChild(wrap);
}

// TypeLine: the lines confirmed so far (checked, ✓/✗ + expected), then one
// input for the next line. Progressive — never tracks more than the server's
// own `results` tells us; the last line's check doubles as the closing
// `feedback` (see `submitTypeLine`), so this only renders the in-progress state.
function renderTypeLine(a) {
  const wrap = el("div", "reveal");
  typelineChecked.forEach(r => checkedLine(wrap, r));
  a.appendChild(wrap);
  const inputs = el("div", "inputs");
  const inp = el("input", "field");
  inp.type = "text"; inp.autocomplete = "off"; inp.spellcheck = false;
  inp.addEventListener("keydown", e => { if (e.key === "Enter") { e.preventDefault(); submitTypeLine(inp.value); } });
  inputs.appendChild(inp);
  a.appendChild(inputs);
  inp.focus();
}

// The options after answering: correct in green, a wrong pick in red.
function renderChoiceFeedback(a) {
  a.classList.add("choices");
  const wrap = el("div", "options");
  state.choices.forEach((opt, i) => {
    let cls = "option";
    if (i === feedback.correct) cls += " correct";
    else if (i === feedback.chosen) cls += " wrong";
    else cls += " dim";
    const row = el("div", cls);
    row.appendChild(el("span", "num", String(i + 1)));
    const text = el("span", "opt");
    if (state.choice_runs && state.choice_runs[i]) appendRuns(text, state.choice_runs[i]);
    else text.textContent = opt;
    row.appendChild(text);
    wrap.appendChild(row);
  });
  a.appendChild(wrap);
}

// Explain (understanding) cards: before reveal a free textarea (optional); after
// reveal your answer beside the key points (the back lines). Self-graded — the
// typed text never leaves the client.
function renderExplain(a) {
  if (revealed === 0) {
    const ta = el("textarea", "explain-input");
    ta.placeholder = "Type your answer… (Shift+Enter to reveal)";
    ta.rows = 3;
    ta.value = explainInput;
    ta.addEventListener("input", () => { explainInput = ta.value; });
    ta.addEventListener("keydown", e => {
      // Enter inserts a newline (compose freely); Shift+Enter reveals. Stop the
      // event here so the same keypress doesn't also reach the document handler
      // and submit the (now visible) checklist in one go.
      if (e.key === "Enter" && e.shiftKey) { e.preventDefault(); e.stopPropagation(); explainReveal(); }
    });
    a.appendChild(ta);
    setTimeout(() => ta.focus(), 0);
    return;
  }
  if (explainInput.trim()) {
    a.appendChild(el("div", "explain-label", "your answer"));
    a.appendChild(el("div", "explain-answer", explainInput));
  }
  // With cached key points, the reveal is the same green ▸ list a trace walk
  // shows — but you walk it top to bottom marking each yes/no, and the coverage
  // derives the grade. Otherwise show the plain back lines.
  if (hasKeypoints()) {
    if (marks.length !== state.keypoints.length) marks = state.keypoints.map(() => undefined);
    // The authored answer (the ground truth) first, then the checklist of the
    // claims it makes — the key points are a decomposition, not a replacement.
    a.appendChild(el("div", "explain-label", "the answer"));
    const ans = el("div", "reveal");
    for (let i = 0; i < state.card.back.length; i++) {
      const answer = el("div", "answer");
      if (state.card.back_runs && state.card.back_runs[i]) appendRuns(answer, state.card.back_runs[i]);
      else answer.textContent = state.card.back[i];
      ans.appendChild(answer);
    }
    a.appendChild(ans);
    appendKeypointList(a, {
      keypoints: state.keypoints,
      keypointRuns: state.keypoint_runs,
      marks,
      cursor: kpCur,
      onClick: clickKeypoint,
    });
    return;
  }
  a.appendChild(el("div", "explain-label", "your answer should cover"));
  const pts = el("div", "reveal");
  for (let i = 0; i < state.card.back.length; i++) {
    const point = el("div", "answer");
    point.appendChild(document.createTextNode("• "));
    if (state.card.back_runs && state.card.back_runs[i]) appendRuns(point, state.card.back_runs[i]);
    else point.appendChild(document.createTextNode(state.card.back[i]));
    pts.appendChild(point);
  }
  a.appendChild(pts);
}
// Walk the key-point list: mark the current point yes/no and advance; move the
// cursor; click toggles a point (and lands the cursor on it).
function answerKeypoint(val) { marks[kpCur] = val; kpCur = Math.min(kpCur + 1, marks.length - 1); fillBottom(); renderLegend(); }
function moveKeypoint(d) { kpCur = Math.max(0, Math.min(kpCur + d, marks.length - 1)); fillBottom(); }
function clickKeypoint(i) { kpCur = i; marks[i] = marks[i] === true ? false : true; fillBottom(); renderLegend(); }
function keypointsYes() { return marks.filter(m => m === true).length; }
function keypointsAnswered() { return marks.length > 0 && marks.every(m => m !== undefined); }
// Display-only mirror of the lib's keypoint_grade (the server stays authoritative
// on submit) — recomputed each render, so re-toggling a point updates the verdict.
function keypointVerdict() {
  const yes = keypointsYes(), total = state.keypoints.length;
  return (total === 0 || yes >= total) ? "passed" : yes === 0 ? "failed" : "partly";
}
// Submit: the server derives failed/partly/passed from how many points were
// covered (the one keypoint_grade rule, in the lib). Unanswered = not covered.
function submitKeypoints() {
  api("/api/grade", post({ covered: keypointsYes(), total: state.keypoints.length })).then(apply).catch(() => load());
}
function explainReveal() {
  const ta = document.querySelector(".explain-input");
  if (ta) explainInput = ta.value;
  revealed = backCount();
  fillBottom();
  renderLegend();
}

// ── draw input ──────────────────────────────────────────────────────
// The canvas you draw/handwrite on, with Pen · Eraser · Undo · Clear. Strokes
// live in `drawStrokes`; the drawing is ephemeral — snapshotted on reveal for
// side-by-side comparison, never persisted or sent to the server.
const ERASER_WIDTH = 40;                        // eraser stroke width, and the diameter of its cursor ring
function renderDrawCanvas(a) {
  const wrap = el("div", "draw-wrap");
  const canvas = el("canvas", "draw-canvas");
  wrap.appendChild(canvas);
  const ring = el("div", "eraser-ring");        // shows the eraser's size/position; hidden until the eraser is over the canvas
  ring.style.width = ring.style.height = ERASER_WIDTH + "px";
  wrap.appendChild(ring);
  const tools = el("div", "draw-tools");
  const pen = drawToolBtn("Pen", () => setDrawTool("pen"));
  const erase = drawToolBtn("Eraser", () => setDrawTool("erase"));
  pen.classList.toggle("on", drawTool === "pen");
  erase.classList.toggle("on", drawTool === "erase");
  tools.appendChild(pen);
  tools.appendChild(erase);
  tools.appendChild(drawToolBtn("Undo", drawUndo));
  tools.appendChild(drawToolBtn("Clear", drawClear));
  wrap.appendChild(tools);
  a.appendChild(wrap);
  setupDrawCanvas(canvas, ring);
}
function drawToolBtn(text, onClick) {
  const b = el("button", "draw-tool", text);
  b.type = "button";
  b.addEventListener("click", e => { e.preventDefault(); e.stopPropagation(); onClick(); });
  return b;
}
function setDrawTool(t) { drawTool = t; render(); }
function drawUndo() { drawStrokes.pop(); redrawStrokes(); }
function drawClear() { drawStrokes = []; redrawStrokes(); }

// Size the canvas to its box (crisp under devicePixelRatio), wire pointer
// drawing (pen/touch/mouse), and replay existing strokes.
function setupDrawCanvas(canvas, ring) {
  drawCanvasEl = canvas;
  const dpr = window.devicePixelRatio || 1;
  const rect = canvas.getBoundingClientRect();
  canvas.width = Math.max(1, Math.round(rect.width * dpr));
  canvas.height = Math.max(1, Math.round(rect.height * dpr));
  const ctx = canvas.getContext("2d");
  ctx.scale(dpr, dpr);
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  canvas._ctx = ctx;
  // In eraser mode the ring stands in for the cursor; the pen keeps a crosshair.
  canvas.style.cursor = drawTool === "erase" ? "none" : "crosshair";
  redrawStrokes();
  let cur = null;
  const pos = e => { const r = canvas.getBoundingClientRect(); return { x: e.clientX - r.left, y: e.clientY - r.top }; };
  const moveRing = e => {
    if (drawTool !== "erase") { ring.style.display = "none"; return; }
    const p = pos(e);
    ring.style.left = p.x + "px";
    ring.style.top = p.y + "px";
    ring.style.display = "block";
  };
  const hideRing = () => { ring.style.display = "none"; };
  canvas.addEventListener("pointerdown", e => {
    e.preventDefault();
    try { canvas.setPointerCapture(e.pointerId); } catch (err) {}
    cur = { tool: drawTool, points: [pos(e)] };
    drawStrokes.push(cur);
    moveRing(e);
  });
  canvas.addEventListener("pointermove", e => {
    moveRing(e);
    if (!cur) return;
    cur.points.push(pos(e));
    drawSeg(ctx, cur);
  });
  canvas.addEventListener("pointerenter", moveRing);
  const end = () => { cur = null; };
  canvas.addEventListener("pointerup", end);
  canvas.addEventListener("pointercancel", () => { end(); hideRing(); });
  canvas.addEventListener("pointerleave", () => { end(); hideRing(); });
}
// Ink is the theme's --ink; the eraser cuts pixels with destination-out.
function drawStyle(ctx, tool) {
  ctx.globalCompositeOperation = tool === "erase" ? "destination-out" : "source-over";
  ctx.strokeStyle = getComputedStyle(document.documentElement).getPropertyValue("--ink").trim() || "#e6e6e6";
  ctx.lineWidth = tool === "erase" ? ERASER_WIDTH : 2.5;
}
// Draw the newest segment (incremental, so live strokes are smooth).
function drawSeg(ctx, s) {
  const p = s.points;
  if (p.length < 2) return;
  drawStyle(ctx, s.tool);
  const a = p[p.length - 2], b = p[p.length - 1];
  ctx.beginPath(); ctx.moveTo(a.x, a.y); ctx.lineTo(b.x, b.y); ctx.stroke();
}
// Replay every stroke onto a cleared canvas (undo / clear).
function redrawStrokes() {
  const canvas = drawCanvasEl;
  if (!canvas || !canvas._ctx) return;
  const ctx = canvas._ctx, dpr = window.devicePixelRatio || 1;
  ctx.globalCompositeOperation = "source-over";
  ctx.clearRect(0, 0, canvas.width / dpr, canvas.height / dpr);
  for (const s of drawStrokes) {
    for (let i = 1; i < s.points.length; i++) {
      drawStyle(ctx, s.tool);
      const a = s.points[i - 1], b = s.points[i];
      ctx.beginPath(); ctx.moveTo(a.x, a.y); ctx.lineTo(b.x, b.y); ctx.stroke();
    }
  }
}
// Reveal: freeze the drawing (kept on screen to self-check against the answer),
// then reveal. Use max(1, …) so an image-only answer (no back lines) still reveals.
function drawReveal() {
  drawSnapshot = drawCanvasEl ? drawCanvasEl.toDataURL() : null;
  revealed = Math.max(1, backCount());
  render();
}
function frozenDrawImg(dataUrl) {
  const wrap = el("div", "draw-frozen");
  const img = el("img", null);
  img.src = dataUrl;
  img.alt = "your drawing";
  wrap.appendChild(img);
  return wrap;
}

// A reshaped multi-item answer (the `format` augment's list) reveals with
// bullets. Authored physical lines remain available for line-reveal and typing,
// but ordinary answers use `back_units`, where Markdown soft wraps are spaces.
function isReshapedList(c) { return !!(c && c.reshaped && c.back.length > 1); }
function appendAnswerUnits(sec, units) {
  for (const unit of units || []) {
    if (unit.kind === "sentence") {
      const answer = el("div", "answer");
      if (unit.runs) appendRuns(answer, unit.runs); else answer.textContent = unit.text || "";
      sec.appendChild(answer);
    } else if (unit.kind === "code") {
      const pre = el("pre", "code-block");
      pre.textContent = (unit.lines || []).join("\n");
      sec.appendChild(pre);
    } else if (unit.kind === "checklist") {
      appendChecklist(sec, unit.items);
    }
  }
}

// Fill the answer region for reveal modes (flip / line / choice fallback).
// The acquire view: a never-seen card shown answer-first, so you read it before
// it's ever quizzed. One key ("Seen") records it; its first quiz comes ~1 min later.
// Only a reshaped list wants a flush-left block: its lines are steps or bullets.
function leftAlignAnswer(c) { return isReshapedList(c); }
function fillAcquire(a) {
  if (!a) return;
  const c = state.card;
  const sec = el("div", "reveal" + (leftAlignAnswer(c) ? " list" : ""));
  if (state.mode === "line") appendReveal(sec, c.back, c.back_runs, false);
  else if (isReshapedList(c)) appendReveal(sec, c.back, c.back_runs, true);
  else appendAnswerUnits(sec, c.back_units);
  a.appendChild(sec);
  appendImages(a, c.images_back);
  a.appendChild(el("div", "acquire-hint", "new card — you'll be quizzed on it in about a minute."));
}

function fillAnswer(a) {
  if (!a) return;
  // fillBottom already cleared the region and added the mode badge; don't wipe it.
  if (revealed === 0) return; // stays empty until revealed

  const c = state.card;
  const shown = state.mode === "line" ? Math.min(revealed, c.back.length) : c.back.length;
  const sec = el("div", "reveal" + (leftAlignAnswer(c) ? " list" : ""));
  if (state.mode === "line") {
    appendReveal(sec, c.back.slice(0, shown), c.back_runs && c.back_runs.slice(0, shown), false);
  } else if (isReshapedList(c)) {
    appendReveal(sec, c.back, c.back_runs, true);
  } else {
    appendAnswerUnits(sec, c.back_units);
  }
  if (state.mode === "line" && shown < c.back.length) sec.appendChild(el("div", "answer pending", "···"));
  a.appendChild(sec);
  // Attach the answer image to the region itself (a flex column), not to the
  // `.reveal` block, so it can be bounded by the region and scaled to fit.
  appendImages(a, c.images_back);
}

// A card side's images render as ordered blocks; `im` is a `{ src, alt }` from
// the card's `images` / `images_back` list, its src a server `/img/<key>` URL.
function appendImages(parent, images) {
  for (const im of (images || [])) parent.appendChild(cardImage(im));
}
function cardImage(im) {
  const img = el("img", "card-img");
  img.src = im.src;
  img.alt = im.alt || "";
  return img;
}

function renderLegend() {
  legend.innerHTML = "";
  clearLegendSides();
  if (feedback) {
    if (isAcquire()) {
      chip("Seen", "primary", acquire, label(KEYS.reveal)); // a pick acknowledges, never grades
      chip("Ask tutor", "ask", tutor.show, label(KEYS.ask), legendRight); // answer is showing: tutor allowed
    } else if (isRecognizeMc()) {
      if (feedback.passed) {
        // A correct Recognize pick: Next commits it; the quiet "I guessed"
        // override (also bound to the failed key) lets an honest guess demote
        // itself instead — both map to /api/grade, never an auto-continue, so
        // the learner always has the last word.
        chip("Next", "primary", () => grade("passed"), label(KEYS.reveal));
        chip("I guessed", "quiet", () => grade("failed"), label(KEYS.failed));
        chip("Ask tutor", "ask", tutor.show, label(KEYS.ask), legendRight);
      } else {
        // A wrong pick: the correct option is already highlighted on screen
        // (renderChoiceFeedback) — Continue is the only action, and it grades
        // the miss (there's no guess left to walk back). Ask tutor is offered
        // here too: "why is the highlighted option right, not the one I picked?"
        chip("Continue", "primary", () => grade("failed"), label(KEYS.reveal));
        chip("Ask tutor", "ask", tutor.show, label(KEYS.ask), legendRight);
      }
    } else {
      // A typed check's (or TypeLine's closing) result: pure evidence — the
      // learner grades it themselves, same three-way as any other reveal.
      chip("Missed it", "failed", () => grade("failed"), label(KEYS.failed));
      chip("Partly", "partly", () => grade("partly"), label(KEYS.partly));
      chip("Got it", "passed", () => grade("passed"), label(KEYS.passed));
      chip("Ask tutor", "ask", tutor.show, label(KEYS.ask), legendRight);
    }
  } else if (isAcquire()) {
    if (effectiveDraw()) {
      if (revealed === 0) {
        chip("Reveal", "primary", drawReveal, label(KEYS.reveal)); // reveal freezes your attempt
        chip("Skip", "", skip, label(KEYS.skip));
      } else {
        chip("Seen", "primary", acquire, label(KEYS.reveal));      // ungraded acknowledgment
        chip("Ask tutor", "ask", tutor.show, label(KEYS.ask), legendRight);
      }
    } else if (isAcquireChoice()) {
      chip("Skip", "", skip, label(KEYS.skip));            // options are tappable
    } else if (revealed > 0) {
      chip("Seen", "primary", acquire, label(KEYS.reveal)); // hide⟷show is the corner `h` toggle, not a footer button
      chip("Ask tutor", "ask", tutor.show, label(KEYS.ask), legendRight);
    } else {
      chip("Reveal", "primary", reveal, label(KEYS.reveal));
      chip("Skip", "", skip, label(KEYS.skip));
    }
  } else if (effectiveDraw() && revealed === 0) {
    chip("Reveal", "primary", drawReveal, label(KEYS.reveal));
    chip("Skip", "", skip, label(KEYS.skip));
  } else if (isChoice()) {
    chip("Skip", "", skip, label(KEYS.skip));
  } else if (isInput()) {
    chip("Submit", "primary", submitCheck, "enter");
    chip("Skip", "", skip, label(KEYS.skip));
  } else if (isTypeLine()) {
    chip("Check", "primary", submitCurrentTypeLine, "enter");
    chip("Skip", "", skip, label(KEYS.skip));
  } else if (isExplain() && !fullyRevealed()) {
    chip("Reveal", "primary", explainReveal, "shift+enter");
    chip("Skip", "", skip, label(KEYS.skip));
  } else if (!fullyRevealed()) {
    chip(state.mode === "line" && revealed > 0 ? "Reveal next" : "Reveal", "primary", reveal, label(KEYS.reveal));
    chip("Skip", "", skip, label(KEYS.skip));
  } else if (hasKeypoints()) {
    chip("Yes", "passed", () => answerKeypoint(true), "y");
    chip("No", "failed", () => answerKeypoint(false), "n");
    if (keypointsAnswered()) {
      // Every point judged: the submit button becomes the derived verdict
      // (re-derived each render, so changing a point updates it).
      const v = keypointVerdict();
      chip(v[0].toUpperCase() + v.slice(1), "v-" + v, submitKeypoints, "enter");
    } else {
      const answered = marks.filter(m => m !== undefined).length;
      chip(`Done ${answered}/${state.keypoints.length}`, "primary", submitKeypoints, "enter").disabled = true;
    }
    chip("Ask tutor", "ask", tutor.show, label(KEYS.ask), legendRight);
  } else if (isRecognizeFallback()) {
    // No MC could be built (too few distractors): attempt→reveal, boolean call.
    chip("Knew it", "passed", () => grade("passed"), label(KEYS.passed));
    chip("Not yet", "failed", () => grade("failed"), label(KEYS.failed));
    chip("Ask tutor", "ask", tutor.show, label(KEYS.ask), legendRight);
  } else {
    chip("Missed it", "failed", () => grade("failed"), label(KEYS.failed));
    chip("Partly", "partly", () => grade("partly"), label(KEYS.partly));
    chip("Got it", "passed", () => grade("passed"), label(KEYS.passed));
    chip("Ask tutor", "ask", tutor.show, label(KEYS.ask), legendRight);
  }
  chip("Leave", "", leaveSession, "esc", legendLeft); // pinned bottom-left; return to the deck picker
}

// Terse, approximate phrase for when the next scheduled card comes due, shown
// on an empty session so "Nothing due." says when to return. No seconds, no
// ticking; null when there is no instant or it has already passed.
function nextDueNote(ms) {
  if (ms == null) return null;
  const delta = Number(ms) - Date.now();
  if (delta <= 0) return null;
  const min = Math.round(delta / 60000);
  if (min < 60) return `Next due in ${Math.max(1, min)} min.`;
  const hr = Math.round(delta / 3600000);
  if (hr < 24) return `Next due in ${hr} h.`;
  const days = Math.round(delta / 86400000);
  return days <= 1 ? "Next due tomorrow." : `Next due in ${days} days.`;
}

function renderSummary() {
  const acc = state.reviews > 0 ? Math.round(100 * state.passed / state.reviews) + "%" : "–";
  const wrap = el("div", "summary");
  wrap.appendChild(el("div", "lede", "session complete"));
  // A first pass over a fresh deck is acquire-only: reviews stay 0 while
  // every card was introduced. Say what actually happened.
  const acquired = state.acquired || 0;
  const headline = state.reviews > 0 ? "Nicely charged."
    : acquired > 0 ? "New cards planted."
    : "Nothing due.";
  wrap.appendChild(el("h2", null, headline));
  const row = (label, value) => {
    const r = el("div", "row");
    r.appendChild(el("span", null, label));
    r.appendChild(el("b", null, value));
    wrap.appendChild(r);
  };
  if (acquired > 0) row("introduced", `${acquired}`);
  row("reviewed", `${state.reviews}`);
  if (state.reviews > 0) {
    row("passed / failed", `${state.passed} / ${state.failed}`);
    row("accuracy", acc);
  }
  const dueLeft = state.due_left || 0;
  const newLeft = state.new_left || 0;
  const nextDue = state.reviews === 0 && acquired === 0 ? nextDueNote(state.next_due_ms) : null;
  if (nextDue) {
    wrap.appendChild(el("div", "note", nextDue));
  } else if (dueLeft > 0) {
    wrap.appendChild(el("div", "note", `${dueLeft} still due.`));
  } else if (newLeft > 0) {
    wrap.appendChild(el("div", "note", `${newLeft} new waiting.`));
  } else if (newLeft === 0 && !state.can_restart) {
    wrap.appendChild(el("div", "note", "Nothing due right now — come back later."));
  }
  const examDue = state.exam_due || [];
  examDue.forEach(name => {
    wrap.appendChild(el("div", "exam-ready", `✦ ${name} is ready for its exam.`));
  });
  stage.appendChild(wrap);
  // An exam-due deck takes the primary action; otherwise the restart chip does.
  examDue.forEach((name, i) => {
    chip(examDue.length === 1 ? "Take the exam" : `Exam: ${name}`, i === 0 ? "primary" : "", () => exam.start(name));
  });
  // Continue the drain when due cards remain; otherwise say the next sitting
  // starts new cards, never an unlabeled button. The waiting count lives in
  // the note: the sitting plants only the capped share, not all of them.
  const restartLabel = dueLeft > 0 ? "Continue"
    : newLeft > 0 ? "Start new"
    : "New session";
  const newSession = chip(restartLabel, examDue.length ? "" : "primary", restart, label(KEYS.restart));
  newSession.disabled = !state.can_restart;
  chip("Leave", "", deselect, "esc");
}

function reveal() {
  revealed = state.mode === "line" ? Math.min(revealed + 1, backCount()) : backCount();
  fillBottom();
  renderLegend();
}
// Acquire only: hide / un-hide the revealed answer so you can self-test the fresh
// encoding (conceal it, try to recall, show it to check) before acknowledging with
// "Seen". Deliberately does ONE thing — flips the answer text's visibility in place.
// It does NOT re-render: the card stays fully revealed, so the note, the footer, the
// answer's own box, everything holds its exact position. Nothing reflows or jumps.
// A first-encounter aid: there's no spaced schedule to lean on yet, so an ordinary
// review has no such toggle — it drills a card by failing it, which brings it back spaced.
function acquireToggle() {
  if (revealed === 0) { reveal(); return; } // first look: reveal (same as the Reveal key)
  answerConcealed = !answerConcealed;
  const a = document.getElementById("ansRegion");
  if (!a) return;
  a.classList.toggle("concealed", answerConcealed); // visibility only — no reflow, no movement
  paintAcqCue(a);
}
// Point the corner cue's glyph/title at what the next press does. A textContent swap
// only — no layout change.
function paintAcqCue(a) {
  const cue = a.querySelector(".cite-toggle");
  if (!cue) return;
  cue.title = answerConcealed ? "show answer" : "hide the answer to self-test";
  const ci = cue.querySelector(".ci");
  if (ci) ci.textContent = answerConcealed ? "⊙" : "⊘";
}
// A tap on the answer region toggles too, but don't hijack a text-selection drag.
function onAcqToggleClick() {
  if (window.getSelection && String(window.getSelection())) return;
  acquireToggle();
}

function chip(text, cls, onClick, key, into) {
  const b = el("button", "chip" + (cls ? " " + cls : ""), text);
  if (key) b.appendChild(el("span", "k", key));
  b.addEventListener("click", onClick);
  (into || legend).appendChild(b);
  return b;
}

// The footer's left (Leave) and right (Ask tutor) zones. Cleared each render;
// the right zone keeps its #score span, which screens set directly.
function clearLegendSides() {
  legendLeft.innerHTML = "";
  legendRight.querySelectorAll(".chip").forEach(c => c.remove());
}

document.addEventListener("keydown", (e) => {
  if (e.altKey || e.metaKey) return; // ctrl handled per-binding below
  // The walk overlay. Ask panel closes on Esc; leave confirmation on Enter/Esc;
  // grade keys on reveal; Esc triggers the leave confirmation (unfinished) or
  // leaves immediately (done). Typing in a textarea is left alone.
  if (walk.isOpen()) {
    walk.handleKey(e);
    return;
  }
  if (!state) return;
  // The exam overlay takes priority. Esc asks to quit (it abandons the exam);
  // while that confirmation is up, Enter quits and Esc keeps going. The textarea
  // handles typing.
  if (exam.isOpen()) {
    exam.handleKey(e);
    return;
  }
  // The browse overlay: step cards (configurable next/prev + arrows/space/g/G),
  // Esc/Backspace leaves. Read-only — no grading.
  if (browsing) {
    if (e.key === "Escape" || e.key === "Backspace") { e.preventDefault(); closeBrowse(); return; }
    if (e.key === "ArrowRight" || e.key === " " || hit(e, BK.next)) { e.preventDefault(); browseGo(1); return; }
    if (e.key === "ArrowLeft" || hit(e, BK.prev)) { e.preventDefault(); browseGo(-1); return; }
    if (e.key === "g" || e.key === "Home") { e.preventDefault(); browsing.i = 0; render(); return; }
    if (e.key === "G" || e.key === "End") { e.preventDefault(); browsing.i = browsing.cards.length - 1; render(); return; }
    return;
  }
  // The augment overlay: Esc/Backspace leaves it (like browse and the picker);
  // the guidance box swallows its own Backspace, so typing there is untouched.
  if (augment.isOpen()) {
    augment.handleKey(e);
    return;
  }
  if (state.phase === "select") {
    picker.handleKey(e);
    return;
  }
  // The ask overlay: Esc closes, the save-note key saves; the textarea handles
  // typing, Enter for a newline, and Shift+Enter to send.
  if (tutor.isOpen()) {
    tutor.handleKey(e);
    return;
  }
  // While the leave prompt is up: Enter confirms leaving, Esc stays; other keys
  // are inert (so a stray Esc can never blow through the guard).
  if (confirmingLeave) {
    if (e.key === "Enter") { e.preventDefault(); deselect(); }
    else if (e.key === "Escape") { e.preventDefault(); cancelLeave(); }
    return;
  }
  // Esc returns to the deck picker (with a confirm when the session isn't done).
  if (e.key === "Escape") { e.preventDefault(); leaveSession(); return; }
  if (state.phase === "done") {
    // Enter takes the primary action (the exam, when a deck is exam due).
    if (state.exam_due && state.exam_due.length && e.key === "Enter") {
      const b = legend.querySelector(".chip.primary");
      if (b) { e.preventDefault(); b.click(); }
      return;
    }
    if (state.can_restart && hit(e, KEYS.restart)) { e.preventDefault(); restart(); }
    return;
  }
  // `s` swaps a cited card between its answer and its source, once answered.
  if ((state.card.citations || []).length && isAnswered() && !e.ctrlKey && e.key.toLowerCase() === "s") {
    e.preventDefault(); toggleCitation(); return;
  }
  // A never-seen card (acquire): recognition pick or recall reveal, then "Seen".
  // Handled before the feedback/grade paths so a pick never grades the card.
  if (isAcquire()) {
    if (hit(e, KEYS.remove)) { e.preventDefault(); remove(); return; }
    // Post-reveal only: once the answer shows (revealed, or a pick's feedback),
    // the tutor is allowed here too, matching review's after-reveal rule.
    if ((revealed > 0 || feedback) && hit(e, KEYS.ask)) { e.preventDefault(); tutor.show(); return; }
    if (effectiveDraw()) {
      if (revealed === 0) {
        if (hit(e, KEYS.skip)) { e.preventDefault(); skip(); return; }
        if (hit(e, KEYS.reveal)) { e.preventDefault(); drawReveal(); return; }
      } else if (hit(e, KEYS.reveal) || e.key === "Enter" || e.key === " ") {
        e.preventDefault(); acquire();
      }
      return;
    }
    if (isAcquireChoice()) {
      if (!feedback) {
        if (hit(e, KEYS.skip)) { e.preventDefault(); skip(); return; }
        if (hit(e, KEYS.up) || e.key === "ArrowUp") { e.preventDefault(); moveChoiceFocus(-1); return; }
        if (hit(e, KEYS.down) || e.key === "ArrowDown") { e.preventDefault(); moveChoiceFocus(1); return; }
        if (e.key === "Enter" && choiceFocus >= 0 && choiceFocus < state.choices.length) { e.preventDefault(); choose(choiceFocus); return; }
        if (e.key >= "1" && e.key <= "9") {
          const i = +e.key - 1;
          if (i < state.choices.length) { e.preventDefault(); choose(i); }
        }
      } else if (hit(e, KEYS.reveal) || e.key === "Enter" || e.key === " ") {
        e.preventDefault(); acquire();
      }
      return;
    }
    // `h` toggles the answer hidden ⟷ shown, both directions on one key (the
    // source⟷answer swap's principle); space reveals, then acknowledges ("Seen").
    if (e.key.toLowerCase() === "h" && !e.ctrlKey) { e.preventDefault(); acquireToggle(); return; }
    if (revealed === 0) {
      if (hit(e, KEYS.skip)) { e.preventDefault(); skip(); return; }
      if (hit(e, KEYS.reveal)) { e.preventDefault(); reveal(); return; }
    } else if (hit(e, KEYS.reveal) || e.key === "Enter" || e.key === " ") {
      e.preventDefault(); acquire();
    }
    return;
  }
  if (feedback) {
    if (hit(e, KEYS.ask)) { e.preventDefault(); tutor.show(); return; }
    if (hit(e, KEYS.remove)) { e.preventDefault(); remove(); return; }
    if (isRecognizeMc()) {
      // Correct pick: reveal/Enter takes the primary "Next" (passed), and the
      // failed key is the quiet "I guessed" override (demote to failed). Wrong
      // pick: reveal/Enter is "Continue", which just grades the miss — there's
      // no guess to walk back on a pick that was already wrong.
      if (feedback.passed && hit(e, KEYS.failed)) { e.preventDefault(); grade("failed"); return; }
      if (hit(e, KEYS.reveal) || e.key === "Enter") { e.preventDefault(); grade(feedback.passed ? "passed" : "failed"); }
      return;
    }
    // A typed check's (or TypeLine's closing) result: the learner grades it,
    // same three-way keys as any other reveal — no auto-continue.
    if (hit(e, KEYS.failed)) { e.preventDefault(); grade("failed"); }
    else if (hit(e, KEYS.partly)) { e.preventDefault(); grade("partly"); }
    else if (hit(e, KEYS.passed)) { e.preventDefault(); grade("passed"); }
    return;
  }
  // While typing in a field, only Ctrl shortcuts act so plain keys stay text;
  // Enter (submit / check-line) is handled by the field itself.
  if (isInput() || isTypeLine()) {
    if (e.ctrlKey && hit(e, KEYS.remove)) { e.preventDefault(); remove(); }
    else if (e.ctrlKey && hit(e, KEYS.skip)) { e.preventDefault(); skip(); }
    return;
  }
  // Draw before reveal: the canvas takes pointer input, not keys. Enter reveals
  // (freezing the drawing) to match the "Reveal" chip; placed before the explain
  // and generic reveal branches so a flip/explain draw card reveals via drawReveal().
  if (effectiveDraw() && revealed === 0) {
    if (hit(e, KEYS.reveal)) { e.preventDefault(); drawReveal(); }
    else if (hit(e, KEYS.skip)) { e.preventDefault(); skip(); }
    else if (hit(e, KEYS.remove)) { e.preventDefault(); remove(); }
    return;
  }
  // Explain before reveal: the textarea takes plain keys (Enter = newline) and
  // Shift+Enter reveals. The textarea handles Shift+Enter when focused (and stops
  // it there); this covers the case where focus has left the textarea.
  if (isExplain() && !fullyRevealed()) {
    if (e.key === "Enter" && e.shiftKey) { e.preventDefault(); explainReveal(); }
    else if (e.ctrlKey && hit(e, KEYS.remove)) { e.preventDefault(); remove(); }
    else if (e.ctrlKey && hit(e, KEYS.skip)) { e.preventDefault(); skip(); }
    return;
  }
  if (hit(e, KEYS.remove)) { e.preventDefault(); remove(); return; }
  if (isChoice()) {
    if (hit(e, KEYS.skip)) { e.preventDefault(); skip(); return; }
    if (hit(e, KEYS.up) || e.key === "ArrowUp") { e.preventDefault(); moveChoiceFocus(-1); return; }
    if (hit(e, KEYS.down) || e.key === "ArrowDown") { e.preventDefault(); moveChoiceFocus(1); return; }
    if (e.key === "Enter" && choiceFocus >= 0 && choiceFocus < state.choices.length) { e.preventDefault(); choose(choiceFocus); return; }
    if (e.key >= "1" && e.key <= "9") {
      const i = +e.key - 1;
      if (i < state.choices.length) { e.preventDefault(); choose(i); }
    }
    return;
  }
  if (!fullyRevealed()) {
    if (hit(e, KEYS.skip)) { e.preventDefault(); skip(); return; }
    if (hit(e, KEYS.reveal)) { e.preventDefault(); reveal(); return; }
    return;
  }
  if (hit(e, KEYS.ask)) { e.preventDefault(); tutor.show(); return; }
  // The key-point checklist replaces the grade buttons: walk the list top to
  // bottom with y/n (auto-advancing), the review up/down keys or arrows to move, Enter to submit once
  // every point is answered (the server derives the grade from coverage).
  if (hasKeypoints()) {
    if (e.key === "y" || e.key === "Y") { e.preventDefault(); answerKeypoint(true); }
    else if (e.key === "n" || e.key === "N") { e.preventDefault(); answerKeypoint(false); }
    else if (hit(e, KEYS.down) || e.key === "ArrowDown") { e.preventDefault(); moveKeypoint(1); }
    else if (hit(e, KEYS.up) || e.key === "ArrowUp") { e.preventDefault(); moveKeypoint(-1); }
    else if (e.key === "Enter" && keypointsAnswered()) { e.preventDefault(); submitKeypoints(); }
    return;
  }
  // The Recognize fallback (no MC) only ever shows two chips (Knew it / Not
  // yet) — the partly key has no matching chip there, so it's a no-op.
  if (hit(e, KEYS.failed)) { e.preventDefault(); grade("failed"); }
  else if (hit(e, KEYS.partly) && !isRecognizeFallback()) { e.preventDefault(); grade("partly"); }
  else if (hit(e, KEYS.passed)) { e.preventDefault(); grade("passed"); }
});

document.getElementById("kebab").addEventListener("click", (e) => { e.stopPropagation(); syncDrawMenu(); menu.classList.toggle("open"); });
// Opening the burger (or clicking a menu item) must not pull focus off the
// focused deck row — otherwise the picker's row-nav keys go dead until you click
// back into the list. preventDefault on mousedown keeps focus where it is (the
// same trick the focus drawer uses on its cells); the click still fires.
document.getElementById("kebab").addEventListener("mousedown", (e) => e.preventDefault());
menu.addEventListener("mousedown", (e) => e.preventDefault());
menu.addEventListener("click", (e) => e.stopPropagation());
document.getElementById("mAsk").addEventListener("click", () => {
  menu.classList.remove("open");
  // Mirrors the footer/keyboard availability: a walk offers the tutor only once
  // a checkpoint is revealed (nothing to ask about while still predicting).
  if (walk.isOpen()) { if (walk.data().phase === "reveal") tutor.show(); }
  else if (isAnswered()) tutor.show();
});
document.getElementById("mRemove").addEventListener("click", () => { menu.classList.remove("open"); remove(); });
document.getElementById("mPromote").addEventListener("click", () => { menu.classList.remove("open"); promote(); });
const mDraw = document.getElementById("mDraw");
function syncDrawMenu() {
  // Sticky only when the card is *actually* drawing because it's authored draw:
  // gate on effectiveDraw() (flip/explain, non-cloze) so an authored `input:draw`
  // on a non-drawable card can't show a misleading disabled "on" with no canvas.
  const authored = effectiveDraw() && state.input === "draw";
  document.getElementById("mDrawState").textContent = (authored || drawToggle) ? "on" : "off";
  mDraw.disabled = authored;
}
syncDrawMenu();
mDraw.addEventListener("click", () => {
  drawToggle = !drawToggle;
  try { localStorage.setItem("alix-draw", drawToggle ? "1" : "0"); } catch (e) {}
  syncDrawMenu();
  menu.classList.remove("open");
  render(); // show/hide the canvas on the current card
});
document.addEventListener("click", () => menu.classList.remove("open"));


// Show the right menu items for the current screen (picker vs review vs walk).
function setMenuContext(ctx) {
  document.querySelectorAll("#menu .m-picker").forEach((b) => { b.style.display = ctx === "picker" ? "" : "none"; });
  // Ask Tutor is the one .m-review item that also makes sense mid-walk; the
  // rest (Remove card, Promote) are per-deck-card actions a trace checkpoint
  // doesn't have, so they get their own narrower checks below.
  document.querySelectorAll("#menu .m-review").forEach((b) => { b.style.display = (ctx === "review" || ctx === "walk") ? "" : "none"; });
  document.getElementById("mRemove").style.display = ctx === "review" ? "" : "none";
  // Promote is a review action, but only while the current card is virtual
  // (a remediation card) — narrower than the other .m-review items, so it
  // gets its own check on top of the context toggle.
  document.getElementById("mPromote").style.display = ctx === "review" && state.promotable ? "" : "none";
  barNav.style.display = ctx === "picker" ? "" : "none";
}

document.getElementById("mShortcuts").addEventListener("click", () => { menu.classList.remove("open"); sheets.openShortcuts(); });
document.getElementById("mAdd").addEventListener("click", () => { menu.classList.remove("open"); sheets.openAdd(); });
document.getElementById("mShare").addEventListener("click", () => { menu.classList.remove("open"); sheets.openShare(); });
document.getElementById("mReset").addEventListener("click", () => { menu.classList.remove("open"); sheets.openReset(); });
document.getElementById("mDoctor").addEventListener("click", () => { menu.classList.remove("open"); sheets.openDoctor(); });
document.getElementById("mPair").addEventListener("click", () => { menu.classList.remove("open"); sheets.openPair(); });
document.getElementById("mAbout").addEventListener("click", () => { menu.classList.remove("open"); sheets.openAbout(); });

// Load the key bindings first, then the session, and retry on a transient
// failure. A just-started server, or the browser reusing a dead keep-alive
// connection from a killed one, can fail the first request; the picker-keys
// endpoint also falls back to the Vim defaults on an older server. Without the
// retry a failed first request left a blank page until a manual refresh.
// A request that never answers is not a rejection, so the retry below could not
// see it: Promise.all simply never settled and the picker stayed blank until a
// manual reload. Bound the wait so a stranded request becomes a retry.
const BOOT_TIMEOUT_MS = 4000;
function boot() {
  const bounded = (work) => Promise.race([
    work,
    new Promise((_, reject) => setTimeout(() => reject(new Error("boot timed out")), BOOT_TIMEOUT_MS)),
  ]);
  bounded(Promise.all([
    api("/api/keys"),
    api("/api/picker-keys").catch(() => ({})),
    api("/api/ask-info").catch(() => null),
    api("/api/browse-keys").catch(() => null),
  ])).then(([k, pk, ai, bk]) => {
    KEYS = k;
    tutor.setInfo(ai);
    if (bk) BK = bk;
    picker.setKeys(pk);
    document.querySelector("#mRemove .mk").textContent = label(KEYS.remove);
    document.getElementById("mAskKey").textContent = label(KEYS.ask);
    return load();
  }).catch(() => setTimeout(boot, 500));
}
boot();
