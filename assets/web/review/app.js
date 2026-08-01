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
let selectEsc = null; // in the deck-select confirm view, Esc returns to browse
let lastWorkspace = null; // name of the workspace you launched from, to return into it
// The focus drawer's current pick for the deck under the cursor: which topology
// orders the session and (optionally) which one region to drill. `deck` tracks
// the focused deck so launch only applies the pick to that deck. `topoCache`
// memoizes each deck's `/api/deck-drawer` payload across picker re-renders.
let drawerSel = { deck: null, topology: null, region: null };
let topoCache = {};
let explainInput = clientModel.explainInput; // the typed reconstruction on an explain card (client-side)
let marks = clientModel.marks;        // per-key-point yes/no/pending for an explain checklist (client-side)
let kpCur = clientModel.keypointCursor;         // the key point the cursor is on, walked top to bottom
let drawStrokes = clientModel.drawStrokes;    // strokes on a draw card this reveal: [{tool, points:[{x,y}]}]
let drawSnapshot = clientModel.drawSnapshot; // frozen dataURL of the drawing, kept visible during self-grade
let drawTool = clientModel.drawTool;    // "pen" | "erase"
let drawCanvasEl = clientModel.drawCanvas; // the live <canvas> while drawing
// Per-device "Draw answers" preference (wired to the menu in Task 5).
let drawToggle = clientModel.drawToggle;
let asking = false;   // the ask-tutor overlay is open
let askData = { transcript: [], thinking: false, status: null, error: null };
let askInfo = { backend: "claude", model: "default", effort: "default" }; // who answers, from /api/ask-info
// The configured backend's display name ("Claude", "Copilot", …) and the
// shared "X is working…" progress line the exam and augment overlays show:
// one place, so no surface can drift back to a hardcoded backend.
const backendName = () => askInfo.backend.charAt(0).toUpperCase() + askInfo.backend.slice(1);
function workingText(s) {
  if (s < 2) return `${backendName()} is working…`;
  if (s < 90) return `${backendName()} is working… ${s}s`;
  return `${backendName()} is working… ${Math.floor(s / 60)}m ${s % 60}s — this can take a couple of minutes`;
}
let askPoll = null;   // setInterval handle while a reply is pending
let askConfirmingClose = false; // showing the "leave the tutor?" confirmation
let augmentData = null; // the AugmentDto while the Augment screen is open (null otherwise)
let augmentPoll = null; // setInterval handle while a generation is in flight
let augTicked = new Set(); // gap-fill target kinds ticked for the next batch generate
let duePoll = null;   // setInterval handle while the summary waits for a cooling card
let browsing = null;  // {cards, label, i} while the read-only browse overlay is open
let askNeedsStateRefresh = false; // a tutor note changed the current card/checkpoint
let KEYS = {};        // configured review key bindings, from /api/keys
let PK = {};          // configured deck-picker nav keys, from /api/picker-keys
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
// Bumped on every manual refresh and appended to /img/ URLs: the icon URLs
// are otherwise stable across regenerations, so the browser would keep the
// old bytes for the lifetime of the app.
let iconNonce = 0;
const refreshDecks = () => { iconNonce = Date.now(); replayLogo(); renderSelect(); };
navRefresh.addEventListener("click", refreshDecks);
const menu = document.getElementById("menu");

function api(path, options) { return apiClient.request(path, options, validatorFor(path)); }
function post(body) { return apiClient.postOptions(body); }
function withToken(path) { return apiClient.withToken(path); }

const exam = createExam({
  api,
  post,
  rememberLaunch,
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

const walk = createWalk({
  api,
  fetchApi: apiClient.fetch,
  post,
  rerender: render,
  applyStudy: apply,
  sessionStorage,
  examStart: exam.start,
  tutor: {
    isOpen: () => asking,
    open: openAsk,
    close: closeAsk,
    render: renderAsk,
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
// A trace picked from the picker returns a WalkDto (kind "walk"); a fact deck
// returns a review StateDto (kind "review"). `isWalk` is the single place the two
// responses are told apart — by the `kind` tag, not by sniffing phase values.
function isWalk(s) {
  return !!s && s.kind === "walk";
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

// The depth menu's cram tick-box — a per-launch choice (plain Learn never
// crams), reset to off every time the menu opens.
let cramOn = false;

// The picker's focused row (hoisted out of `renderList`'s own closure, which
// resets it to null on every render — see there — so the kebab menu's
// Share/Reset actions, wired outside that closure, can still read it).
let focusedEl = null;

// `depth` picks a session depth (Recognize/Recall/Reconstruct) explicitly;
// omitted, the server resolves the deck's own remembered last depth.
function select(name, topology, region, depth, cram) {
  rememberLaunch(name);
  api("/api/select", post({ deck: name, topology: topology || null, region: region || null, depth: depth || null, cram: !!cram })).then(s => {
    if (isWalk(s)) {
      walk.open(s);
    } else {
      apply(s);
    }
  }).catch(() => notice("could not start the session — the server log has details"));
}
// Remember the deck just launched so the picker re-lands the cursor on it when it
// re-opens (the selection shouldn't move while the user is away). Survives both an
// in-page return (review/exam/browse) and a page-reload return (walk).
function rememberLaunch(name) { if (name) sessionStorage.setItem("alix.lastDeck", name); }
// Browse a deck read-only: the server builds the card list and returns it; open
// the in-page browse overlay (no page nav). Leaving returns to the picker, which
// re-lands on this deck (rememberLaunch + lastWorkspace).
function browseDeck(it, wsName) {
  rememberLaunch(it.name);
  lastWorkspace = wsName || null;
  api("/api/browse", post({ deck: it.name })).then(d => { browsing = { cards: d.cards, label: d.label, i: 0 }; render(); });
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
  if (augmentData) { renderAugment(); return; }
  const screen = currentScreen({ ...clientModel, state, browsing, walk: walk.data() });
  if (screen === "walk") { walk.render(); return; }
  if (screen === "browse") { renderBrowse(); return; }
  if (screen === "picker") { renderSelect(); return; }

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

  if (asking) { renderAsk(); return; }
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
function openAsk() {
  asking = true;
  askConfirmingClose = false;
  askNeedsStateRefresh = false;
  render(); // builds the panel once (entrance animation runs once)
  const ep = walk.isOpen() ? "/api/walk/ask" : "/api/ask";
  api(ep).then(d => { askData = d; if (asking) syncAsk(); }); // pick up any prior transcript
}
function closeAsk() {
  // Leaving with an unsaved conversation gets the same pause as leaving a
  // session: the transcript survives on this card, but moving on to the next
  // one drops it before it became a note or a card.
  if (!askConfirmingClose && askData && askData.transcript && askData.transcript.length) {
    askConfirmingClose = true;
    renderAskLeaveConfirm();
    return;
  }
  askConfirmingClose = false;
  asking = false;
  stopAskPoll();
  if (walk.isOpen() && askNeedsStateRefresh) {
    // Re-pull the walk state (a saved note may have landed on a checkpoint).
    api("/api/walk").then(d => { walk.replace(d); render(); }).catch(render);
  } else if (!walk.isOpen() && askNeedsStateRefresh) {
    // A note saved during the chat now lives on the server's card; re-pull the
    // state so it shows on return without a manual reload. Keep the reveal
    // position (`revealed`/`feedback` are client-side and the card can't change
    // while the modal ask panel is open), so we don't reset the card to its front.
    api("/api/state").then(s => { state = s; render(); }).catch(render);
  } else render();
}
function sendAsk(text) {
  const q = (text || "").trim();
  if (!q || askData.thinking) return;
  const ta = document.querySelector(".ask-input");
  if (ta) ta.value = ""; // the textarea persists across syncAsk; clear the sent question
  const ep = walk.isOpen() ? "/api/walk/ask" : "/api/ask";
  api(ep, post({ question: q })).then(d => { askData = d; if (asking) syncAsk(); startAskPoll(); }).catch(() => load());
}
// Save-note and Make-this-a-card both distill a completed exchange into
// something durable, so both need at least one tutor answer and no in-flight
// turn. Their footer chips are disabled (and these guards no-op) until then.
function canDistill() { return !askData.thinking && askData.transcript.length > 0; }
function saveAskNote() {
  if (!canDistill()) return;
  askNeedsStateRefresh = true;
  const ep = walk.isOpen() ? "/api/walk/ask/note" : "/api/ask/note";
  api(ep, post({})).then(d => { askData = d; if (asking) syncAsk(); startAskPoll(); }).catch(() => load());
}
// Distill the conversation so far into one draft card, surfaced on askData.draft
// once the poll picks up the tutor's reply. Adult-only (no walk equivalent).
function draftCard() {
  if (!canDistill()) return;
  api("/api/ask/card/draft", post({}))
    .then(d => { askData = d; if (asking) syncAsk(); startAskPoll(); })
    .catch(() => { askData = { ...askData, status: "couldn't draft a card" }; if (asking) syncAsk(); });
}
// Mint the edited draft as a free-standing card. A 422 (duplicate/malformed) or
// any other non-2xx rejects `api()`, so the failure path always shows a status
// instead of silently doing nothing.
function createCard(front, back) {
  api("/api/ask/card/create", post({ front, back }))
    .then(() => { askData = { ...askData, draft: null, status: "card added" }; if (asking) syncAsk(); })
    .catch(() => { askData = { ...askData, status: "couldn't add that card" }; if (asking) syncAsk(); });
}
function cancelDraft() { askData = { ...askData, draft: null }; if (asking) syncAsk(); }
// Loop the header logo while any backend/server call is in flight — a calm loader
// that complements the inline spinners; rest it otherwise.
function updateBusy() {
  var logo = document.querySelector(".brand alix-logo");
  if (logo) logo.toggleAttribute("loop", !!(askPoll || exam.isPolling()));
}
// Replay the header logo's reveal once — the reload button and the `r` key.
function replayLogo() {
  var logo = document.querySelector(".brand alix-logo");
  if (logo && logo.replay) logo.replay();
}
// Render the dependency-tree prefix as CSS-drawn guides instead of box-drawing
// glyphs. The prefix is 3-char groups ("│  "/"   " ancestors, "├─ "/"└─ " branch);
// each group becomes a fixed-width cell whose CSS vertical extends past the row gap,
// so adjacent rows' verticals connect into continuous lines (glyphs can't span it).
function treeGuides(prefix) {
  const box = el("span", "tree");
  for (let i = 0; i + 3 <= prefix.length; i += 3) {
    const c = prefix[i];
    const cls = c === "│" ? "line" : c === "├" ? "tee" : c === "└" ? "ell" : "empty";
    box.appendChild(el("span", "guide " + cls));
  }
  return box;
}
function startAskPoll() {
  stopAskPoll();
  askPoll = setInterval(() => {
    const ep = walk.isOpen() ? "/api/walk/ask" : "/api/ask";
    api(ep).then(d => { askData = d; if (!d.thinking) { stopAskPoll(); refreshAskInfo(); } if (asking) syncAsk(); });
  }, 400);
  updateBusy();
}
function stopAskPoll() { if (askPoll) { clearInterval(askPoll); askPoll = null; } updateBusy(); }
// An unpinned backend only names its model once it has answered, so pick that
// up the first time and stop showing "default".
function refreshAskInfo() {
  if (askInfo.model !== "default") return;
  api("/api/ask-info").then(ai => { if (ai) { askInfo = ai; if (asking) syncAsk(); } }).catch(() => {});
}

// (Re)fill just the transcript log from askData.
function fillAskLog(log) {
  // Skip the rebuild when nothing visible changed (the poll ticks every 400ms
  // while thinking): a rebuilt <alix-logo> would restart its loop each tick,
  // and the scroll position would keep snapping.
  const sig = JSON.stringify([askData.transcript.length, askData.thinking, askData.status, askData.error]);
  if (log._sig === sig) return;
  log._sig = sig;
  log.innerHTML = "";
  for (const ex of askData.transcript) {
    log.appendChild(el("div", "ask-q", ex.q));
    log.appendChild(el("div", "ask-a", ex.a));
  }
  if (askData.thinking) {
    // The looping alix logo as the thinking indicator (the header's does too,
    // but this one sits where you're looking).
    const t = el("div", "ask-thinking");
    const logo = document.createElement("alix-logo");
    logo.setAttribute("height", "18");
    logo.setAttribute("loop", "");
    t.appendChild(logo);
    t.appendChild(document.createTextNode("Thinking…"));
    log.appendChild(t);
  }
  if (askData.status) log.appendChild(el("div", "ask-status", askData.status));
  if (askData.error) log.appendChild(el("div", "ask-error", askData.error));
  if (!askData.transcript.length && !askData.thinking && !askData.status && !askData.error)
    log.appendChild(el("div", "ask-hint", walk.isOpen() ? "Ask the tutor about this step." : "Ask the tutor about this card."));
  // Keep the newest exchange in view; the card + conversation scroll together
  // now (the card sticks), so scroll their shared container, not just the log.
  const sc = log.closest(".ask-scroll") || log;
  sc.scrollTop = sc.scrollHeight;
}

// The editable front/back form shown once the tutor distills a draft
// (askData.draft). Rebuilt fresh on every call (same idiom as fillAskLog's
// log rebuild), so renderAsk and syncAsk share one source of truth for it.
function buildDraftBox() {
  if (!askData.draft) return null;
  const box = el("div", "draft-box");
  box.appendChild(el("div", "draft-label", "New card (edit, then Add):"));
  const frontField = el("input", "draft-front");
  frontField.value = askData.draft.front;
  box.appendChild(frontField);
  const backField = el("textarea", "draft-back");
  backField.value = askData.draft.back.join("\n");
  backField.rows = Math.max(2, askData.draft.back.length);
  box.appendChild(backField);
  const actions = el("div", "draft-actions");
  chip("Add", "primary", () => {
    const back = backField.value.split("\n").map(s => s.trim()).filter(Boolean);
    createCard(frontField.value.trim(), back);
  }, "", actions);
  chip("Cancel", "", cancelDraft, "", actions);
  box.appendChild(actions);
  return box;
}

// Update the open panel in place — no rebuild, so the entrance animation
// doesn't re-run while we poll and the textarea keeps its focus/value.
function syncAsk() {
  const wrap = document.querySelector(".ask-panel");
  if (!wrap) { render(); return; }
  fillAskLog(wrap.querySelector(".ask-log"));
  const oldDraftBox = wrap.querySelector(".draft-box");
  if (oldDraftBox) oldDraftBox.remove();
  const draftBox = buildDraftBox();
  if (draftBox) wrap.insertBefore(draftBox, wrap.querySelector(".ask-input"));
  const input = wrap.querySelector(".ask-input");
  if (input) {
    input.disabled = askData.thinking;
    // Once the reply lands, focus the box so a follow-up can be typed
    // immediately (the poll stops here, so this fires once per answer).
    if (!askData.thinking) input.focus();
  }
  const send = legend.querySelector(".chip.primary");
  if (send) send.disabled = askData.thinking;
  // Enable Make this a note / Make this a card once an answer lands (or disable again
  // when a follow-up is in flight). The legend is built once, so update here.
  legend.querySelectorAll(".chip.distill").forEach(c => { c.disabled = !canDistill(); });
}

// `subject` is optional: null = use the current review card; an object with
// {q, qRuns, items, itemRuns} = the walk checkpoint prompt and key points.
function renderAsk(subject) {
  const wrap = el("div", "ask-panel");
  // Mono eyebrow header ("ASK TUTOR · card-scoped" / "· step-scoped"), with
  // which model is answering (and its effort) alongside it — the tutor uses
  // the CLI default unless `[ask]` pins one, not the stronger model that
  // built the deck.
  const head = el("div", "ask-head");
  const title = el("span");
  title.appendChild(el("span", "ask-eyebrow", "ASK TUTOR"));
  title.appendChild(el("span", "ask-scope", subject ? "· step-scoped" : "· card-scoped"));
  head.appendChild(title);
  head.appendChild(el("span", "ask-model", `${askInfo.backend} · model: ${askInfo.model} · effort: ${askInfo.effort}`));
  wrap.appendChild(head);

  // The card/checkpoint and the conversation share one scroll region: the subject
  // sticks to the top as a reference while the conversation scrolls under it.
  const scroll = el("div", "ask-scroll");
  if (subject) {
    // Walk: show the checkpoint prompt + key points.
    const ref = el("div", "ask-card");
    const prompt = el("div", "ask-card-q");
    appendRunsOrText(prompt, subject.q, subject.qRuns);
    ref.appendChild(prompt);
    for (let i = 0; i < (subject.items || []).length; i++) {
      const answer = el("div", "ask-card-a");
      answer.appendChild(document.createTextNode("▸ "));
      appendRunsOrText(answer, subject.items[i], subject.itemRuns && subject.itemRuns[i]);
      ref.appendChild(answer);
    }
    scroll.appendChild(ref);
  } else {
    // Review: show the current card's front + answer lines.
    const c = state.card;
    if (c) {
      const ref = el("div", "ask-card");
      const front = el("div", "ask-card-q");
      if (c.front_runs) appendRuns(front, c.front_runs); else front.textContent = c.front;
      ref.appendChild(front);
      for (let i = 0; i < (c.context || []).length; i++) {
        ref.appendChild(contextLine(c.context[i], c.context_runs && c.context_runs[i], "ask-card-ctx"));
      }
      for (let i = 0; i < c.back.length; i++) {
        const answer = el("div", "ask-card-a");
        if (c.back_runs && c.back_runs[i]) appendRuns(answer, c.back_runs[i]);
        else answer.textContent = c.back[i];
        ref.appendChild(answer);
      }
      scroll.appendChild(ref);
    }
  }
  const log = el("div", "ask-log");
  fillAskLog(log);
  scroll.appendChild(log);
  wrap.appendChild(scroll);

  const draftBox = buildDraftBox();
  if (draftBox) wrap.appendChild(draftBox);

  const input = el("textarea", "ask-input");
  input.placeholder = subject ? "Ask about this step… (Shift+Enter to send)" : "Ask about this card… (Shift+Enter to send)";
  input.rows = 2;
  input.disabled = askData.thinking;
  input.addEventListener("keydown", e => {
    // Enter inserts a newline (compose freely); Shift+Enter sends.
    if (e.key === "Enter" && e.shiftKey) { e.preventDefault(); sendAsk(input.value); }
  });
  wrap.appendChild(input);
  stage.appendChild(wrap);

  renderAskFooter(input);
  if (!askData.thinking) input.focus();
}
// The ask panel's footer chips — split out so Stay (below) can restore them
// after the leave confirmation replaced the footer.
function renderAskFooter(input) {
  legend.innerHTML = "";
  const send = chip("Send", "primary", () => sendAsk(input.value), "shift+enter");
  send.disabled = askData.thinking;
  // Make this a note / Make this a card distill a completed exchange, so they
  // stay disabled (the `distill` class, refreshed on every poll in syncAsk)
  // until a tutor answer exists.
  chip("Make this a note", "distill", saveAskNote, label(KEYS.make_note)).disabled = !canDistill();
  // Adult-only, review-scoped: drafting posts to the review-only draft endpoint,
  // which the walk's tutor session (reviewing = None server-side) can't serve.
  if (!walk.isOpen()) chip("Make this a card", "distill", draftCard, label(KEYS.make_card)).disabled = !canDistill();
  chip("Close", "", closeAsk, "esc");
}
// The leave-the-tutor confirmation, mirroring the session/walk leave prompts:
// it swaps only the footer, so the conversation stays on screen while you decide.
function renderAskLeaveConfirm() {
  legend.innerHTML = "";
  legend.appendChild(el("span", "leave-msg",
    "Leave the tutor? Moving on to the next card drops this conversation. Making a note or a card keeps it."));
  chip("Leave anyway", "again", closeAsk);
  chip("Stay", "primary", cancelAskLeave, "esc");
}
function cancelAskLeave() {
  askConfirmingClose = false;
  const input = document.querySelector(".ask-input");
  if (input) { renderAskFooter(input); input.focus(); }
}

// ── AI augment (the picker's "Augment a" action, decks only) ────────────────
// Reports what a deck's augmentation cache holds, fills the gaps (a costed
// background call, polled like the exam), and removes. Generation is one job at
// a time; the page polls /api/augment while it runs.
function openAugment(deck) {
  rememberLaunch(deck);
  augTicked.clear();
  api("/api/augment/open", post({ deck })).then(d => {
    if (d && d.rows) { augmentData = d; render(); }
  });
}
function closeAugment() {
  stopAugmentPoll();
  augTicked.clear();
  api("/api/augment/close", post({})).then(s => { augmentData = null; apply(s); });
}
// A target card's own guidance input text, read when its Generate (or the
// batch) is clicked.
function augGuidance(kind) {
  const i = document.querySelector(`.aug-guide-input[data-kind="${kind}"]`);
  return i ? i.value.trim() : "";
}
function augmentGenerate(target) {
  if (augmentData.busy) return;
  const body = { targets: [{ target, with: augGuidance(target) || null }] };
  api("/api/augment/generate", post(body)).then(d => {
    augmentData = d; refreshAugment(); if (d.busy) startAugmentPoll();
  });
}
// The footer's "Generate selected" action: every ticked gap-fill kind in one
// batch, each carrying its own card's guidance. The server runs them one at a
// time and reports queued/done/failed as it goes (polled the same way as a
// single-target generate).
function augmentGenerateSelected() {
  if (augmentData.busy || !augTicked.size) return;
  const targets = Array.from(augTicked).map(t => ({ target: t, with: augGuidance(t) || null }));
  augTicked.clear();
  api("/api/augment/generate", post({ targets })).then(d => {
    augmentData = d; refreshAugment(); if (d.busy) startAugmentPoll();
  });
  refreshAugment();
}
function augmentRemove(target, topology) {
  api("/api/augment/remove", post({ target, topology: topology || null }))
    .then(d => { augmentData = d; refreshAugment(); });
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
  augmentPoll = setInterval(() => {
    api("/api/augment").then(d => {
      const prev = augmentData;
      augmentData = d;
      if (!d.busy) stopAugmentPoll();
      // Re-render when the in-flight state changed (a job started or finished, so
      // the coverage bars move) or the batch queue/done/failed sets changed;
      // otherwise tick the elapsed counter in place so the spinner's entrance
      // animation doesn't restart (flicker).
      if (!prev || prev.busy !== d.busy || prev.error !== d.error || augBatchSig(prev) !== augBatchSig(d)) refreshAugment();
      else {
        const e = document.querySelector(".exam-elapsed");
        if (e) { e.innerHTML = ""; e.appendChild(el("span", "dot"));
                 e.appendChild(document.createTextNode(augElapsedText(d))); }
      }
    });
  }, 600);
}
function stopAugmentPoll() { if (augmentPoll) { clearInterval(augmentPoll); augmentPoll = null; } }
function augElapsedText(d) {
  return workingText(d.elapsed || 0);
}
// The in-flight spinner cell (reuses the exam's pulsing dot + elapsed styling).
function augBusyCell(d) {
  const sp = el("span", "exam-elapsed");
  sp.appendChild(el("span", "dot"));
  sp.appendChild(document.createTextNode(augElapsedText(d)));
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
  if (!hasGap) augTicked.delete(row.kind);
  const right = el("div", "aug-actions");
  if (hasGap) {
    const cb = el("input", "aug-check");
    cb.type = "checkbox";
    cb.title = "Select for batch generate";
    cb.checked = augTicked.has(row.kind);
    cb.disabled = !!d.busy;
    cb.addEventListener("change", () => {
      if (cb.checked) augTicked.add(row.kind); else augTicked.delete(row.kind);
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
  cb.checked = augTicked.has("topology");
  cb.disabled = !!d.busy;
  cb.addEventListener("change", () => {
    if (cb.checked) augTicked.add("topology"); else augTicked.delete("topology");
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
  cb.checked = augTicked.has("icon");
  cb.disabled = !!d.busy;
  cb.addEventListener("change", () => {
    if (cb.checked) augTicked.add("icon"); else augTicked.delete("icon");
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
  const d = augmentData;
  const btn = document.querySelector(".aug-tools button");
  if (!d || !btn) return;
  const tickable = d.rows.filter(augTickable).map(r => r.kind);
  const allOn = tickable.length > 0 && tickable.every(k => augTicked.has(k));
  btn.textContent = allOn ? "Clear selection" : "Select all";
}
// Builds the augment body (select-all + target cards + the cost footer) into
// `wrap`. Shared by the first mount and the in-place refresh.
function augFillContent(wrap) {
  const d = augmentData;
  if (d.error) wrap.appendChild(el("div", "exam-error", "⚠ " + d.error));
  const tools = el("div", "aug-tools");
  const tickable = d.rows.filter(augTickable).map(r => r.kind);
  const allOn = tickable.length > 0 && tickable.every(k => augTicked.has(k));
  const sa = el("button", "aug-btn ghost", allOn ? "Clear selection" : "Select all");
  sa.disabled = !!d.busy || !tickable.length;
  sa.addEventListener("click", () => {
    const on = tickable.length > 0 && tickable.every(k => augTicked.has(k));
    if (on) augTicked.clear(); else tickable.forEach(k => augTicked.add(k));
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
function renderAugment() {
  const d = augmentData;
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
  if (!existing) { render(); return; }
  const scrollTop = existing.scrollTop;
  const typed = {};
  let focused = null;
  existing.querySelectorAll(".aug-guide-input").forEach(i => {
    if (i.value) typed[i.dataset.kind] = i.value;
    if (i === document.activeElement) focused = i.dataset.kind;
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
  const d = augmentData;
  if (!d) return;
  legend.innerHTML = "";
  const tickedCount = augTicked.size;
  if (tickedCount > 0) {
    let work = 0;
    for (const row of d.rows) if (augTicked.has(row.kind)) work += row.kind === "topology" || row.kind === "icon" ? 1 : Math.max(0, row.eligible - row.covered);
    legend.appendChild(el("span", "aug-work",
      `will run ~${work} generation${work === 1 ? "" : "s"} across ${tickedCount} target${tickedCount === 1 ? "" : "s"}`));
  }
  const genSel = chip(`Generate selected (${tickedCount})`, "primary", augmentGenerateSelected);
  genSel.disabled = tickedCount === 0 || !!d.busy;
  const rmAll = chip("Remove all", "", () => {
    if (confirm("Remove every augmentation for this deck?")) augmentRemove("all");
  });
  rmAll.disabled = !!d.busy;
  chip("Close", "", closeAugment, "esc");
}

// A CLI action while the tab is blurred — `alix receive`, `alix generate`, a
// file dropped into the decks dir — adds decks the open picker can't see. The
// catalog is read fresh from disk on every fetch, so regaining focus in the
// select phase re-scans (same as the ⟳ button). The scan is QUIET: the screen
// repaints only when the catalog actually changed, so a plain alt-tab back
// never visibly refreshes the picker. Overlays and sessions are left alone.
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
  state && state.phase === "select" && !browsing && !exam.isOpen() && !augmentData && !walk.isOpen() && !asking;
window.addEventListener("focus", async () => {
  if (!idleInSelect()) return;
  // An opportunistic re-scan stays quiet on failure too; the visible error
  // state belongs to the deliberate loads (initial, refresh, retry).
  const fresh = await api("/api/decks").catch(() => null);
  if (!fresh) return;
  // Re-check after the await: the user may have started something meanwhile.
  if (idleInSelect() && catalogSignature(fresh) !== lastDecksSignature) {
    renderSelect(fresh);
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
async function renderSelect(preloaded) {
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
      retry.addEventListener("click", () => renderSelect());
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
    if (!it.is_trace && it.state === "examdue" && it.examable) exam.start(it.name);
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
      const img = document.createElement("img");
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
  // back into renderSelect rather than following up with a GET. Sets the
  // one-shot re-land marker (the same trick renderDrill's `back` uses below)
  // so the top list re-focuses this workspace row rather than the first one.
  function submitDeadline(name, date) {
    api("/api/workspace/deadline", post({ name, date }))
      .then(async (d) => {
        sessionStorage.setItem("alix.lastDeck", name);
        await renderSelect(d);
        // Put focus back on the workspace row the change was made from, so
        // set AND clear both resume keyboard flow in place.
        for (const r of deckEl.querySelectorAll(".deckrow")) {
          if (r._open && r._open.name === name) { r.focus(); break; }
        }
      })
      .catch(() => notice("could not update the deadline — the server log has details"));
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
        // PK's per-depth bindings are keyed by the depth names themselves,
        // so PK[d] is each chip's binding (1/2/3 by default). With cram on,
        // every depth is startable — cram serves cards that aren't due.
        for (const d of DEPTHS) {
          const b = el("button", "chip" + (d === it.last_depth ? " primary" : ""), d[0].toUpperCase() + d.slice(1));
          b.appendChild(el("span", "k", label(PK[d])));
          b.disabled = !canDoDepth(it, d) || (!cramOn && !canStartAt(it, f._gated, d));
          b.addEventListener("mousedown", e => e.preventDefault());
          b.addEventListener("click", () => { depthMenuOpen = false; launchDepth(it, f._wsName, f._gated, d); });
          legend.appendChild(b);
        }
        // The cram tick-box: include cards that aren't due (a due card still
        // grades as a normal review; an early pass only re-anchors).
        const cram = el("button", "chip" + (cramOn ? " primary" : ""), (cramOn ? "☑" : "☐") + " cram");
        cram.title = "include cards that aren't due — due cards still count as normal reviews";
        cram.appendChild(el("span", "k", label(PK.cram)));
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
        lv.appendChild(el("span", "k", label(PK.depth)));
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
        br.addEventListener("click", () => browseDeck(f._item, f._wsName));
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
        if (it.examable) ex.addEventListener("click", () => { lastWorkspace = f._wsName || null; exam.start(it.name); });
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
            { duration: DRAWER_MS, easing: DRAWER_EASE, fill: "forwards" }
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
          { duration: DRAWER_MS, easing: DRAWER_EASE }
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
    if (stage._refocusOut) document.removeEventListener("focusout", stage._refocusOut);
    stage._refocusOut = () => {
      if (!stage.contains(wrap)) return; // picker no longer showing
      setTimeout(() => {
        if (!stage.contains(wrap)) return;
        const active = document.activeElement;
        if (active && active !== document.body) return; // focus landed somewhere real
        const v = visibleRows();
        const row = (focusedEl && v.includes(focusedEl)) ? focusedEl : v[0];
        if (row) row.focus();
      }, 0);
    };
    document.addEventListener("focusout", stage._refocusOut);

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
        const d = DEPTHS.find(l => hit(e, PK[l]));
        if (d && f && f._item && canDoDepth(f._item, d) && (cramOn || canStartAt(f._item, f._gated, d))) {
          e.preventDefault(); e.stopPropagation();
          depthMenuOpen = false;
          launchDepth(f._item, f._wsName, f._gated, d);
        } else if (hit(e, PK.cram)) {
          e.preventDefault(); e.stopPropagation(); cramOn = !cramOn; syncPrimary();
        } else if (e.key === "Escape" || hit(e, PK.depth)) {
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
      if (e.key === "ArrowDown" || hit(e, PK.down)) { e.preventDefault(); if (idx < v.length - 1) v[idx + 1].focus(); }
      else if (e.key === "ArrowUp" || hit(e, PK.up)) { e.preventDefault(); if (idx > 0) v[idx - 1].focus(); } // stays on the first row; the filter is only reachable via / or Ctrl-F
      else if (e.key === "g" || e.key === "Home") { e.preventDefault(); if (v.length) v[0].focus(); }
      else if (e.key === "G" || e.key === "End") { e.preventDefault(); if (v.length) v[v.length - 1].focus(); }
      else if (hit(e, PK.filter)) { e.preventDefault(); filter.focus(); }
      else if (e.key === "ArrowRight" || hit(e, PK.open)) { e.preventDefault(); if (drawerCycle) drawerCycle(1); else if (cur && cur._open) renderDrill(cur._open); } // a deck is launched with Enter (the primary chip), not l/→
      else if (e.key === "ArrowLeft" || hit(e, PK.back)) { e.preventDefault(); if (drawerCycle) drawerCycle(-1); } // back-out is Esc/Backspace only; ←/h just steps the drawer
      else if (e.key === "b" && cur && cur._item) { e.preventDefault(); browseDeck(cur._item, cur._wsName); }
      else if (e.key === "a" && cur && (cur._item || cur._open)) { e.preventDefault(); lastWorkspace = cur._wsName || null; openAugment(cur._item ? cur._item.name : cur._open.name); }
      else if (e.key === "d" && cur && cur._open && cur._open.state === "workspace") { e.preventDefault(); deadlinePromptOpen = true; syncPrimary(); }
      else if (hit(e, PK.depth) && hasDepthSplit(cur) && canStart(cur._item, cur._gated)) { e.preventDefault(); depthMenuOpen = true; cramOn = false; syncPrimary(); }
      else if (e.key === "r") { e.preventDefault(); refreshDecks(); } // re-scan the decks (also the ⟳ nav button)
      else if (opts.allowMastered && hit(e, PK.mastered)) { e.preventDefault(); renderMastered(); }
      else if (e.key === "x" && cur && cur._item && cur._item.examable) {
        e.preventDefault(); lastWorkspace = cur._wsName || null; exam.start(cur._item.name);
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
      lede: "mastered \u{1F389} — reopen a deck to cram or re-examine",
      sections: [{ title: null, rows: mastered.map(d => deckRow({ ...d, tree: "" }, false, null, true, false)) }],
      back: renderTop, allowMastered: false,
    });
  }

  // Returning from a session launched inside a workspace/folder re-opens it.
  const reopen = lastWorkspace && workspaces.concat(folders).find(g => g.name === lastWorkspace);
  if (reopen) renderDrill(reopen); else renderTop();
}

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
      chip("Ask tutor", "ask", openAsk, label(KEYS.ask), legendRight); // answer is showing: tutor allowed
    } else if (isRecognizeMc()) {
      if (feedback.passed) {
        // A correct Recognize pick: Next commits it; the quiet "I guessed"
        // override (also bound to the failed key) lets an honest guess demote
        // itself instead — both map to /api/grade, never an auto-continue, so
        // the learner always has the last word.
        chip("Next", "primary", () => grade("passed"), label(KEYS.reveal));
        chip("I guessed", "quiet", () => grade("failed"), label(KEYS.failed));
        chip("Ask tutor", "ask", openAsk, label(KEYS.ask), legendRight);
      } else {
        // A wrong pick: the correct option is already highlighted on screen
        // (renderChoiceFeedback) — Continue is the only action, and it grades
        // the miss (there's no guess left to walk back). Ask tutor is offered
        // here too: "why is the highlighted option right, not the one I picked?"
        chip("Continue", "primary", () => grade("failed"), label(KEYS.reveal));
        chip("Ask tutor", "ask", openAsk, label(KEYS.ask), legendRight);
      }
    } else {
      // A typed check's (or TypeLine's closing) result: pure evidence — the
      // learner grades it themselves, same three-way as any other reveal.
      chip("Missed it", "failed", () => grade("failed"), label(KEYS.failed));
      chip("Partly", "partly", () => grade("partly"), label(KEYS.partly));
      chip("Got it", "passed", () => grade("passed"), label(KEYS.passed));
      chip("Ask tutor", "ask", openAsk, label(KEYS.ask), legendRight);
    }
  } else if (isAcquire()) {
    if (effectiveDraw()) {
      if (revealed === 0) {
        chip("Reveal", "primary", drawReveal, label(KEYS.reveal)); // reveal freezes your attempt
        chip("Skip", "", skip, label(KEYS.skip));
      } else {
        chip("Seen", "primary", acquire, label(KEYS.reveal));      // ungraded acknowledgment
        chip("Ask tutor", "ask", openAsk, label(KEYS.ask), legendRight);
      }
    } else if (isAcquireChoice()) {
      chip("Skip", "", skip, label(KEYS.skip));            // options are tappable
    } else if (revealed > 0) {
      chip("Seen", "primary", acquire, label(KEYS.reveal)); // hide⟷show is the corner `h` toggle, not a footer button
      chip("Ask tutor", "ask", openAsk, label(KEYS.ask), legendRight);
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
    chip("Ask tutor", "ask", openAsk, label(KEYS.ask), legendRight);
  } else if (isRecognizeFallback()) {
    // No MC could be built (too few distractors): attempt→reveal, boolean call.
    chip("Knew it", "passed", () => grade("passed"), label(KEYS.passed));
    chip("Not yet", "failed", () => grade("failed"), label(KEYS.failed));
    chip("Ask tutor", "ask", openAsk, label(KEYS.ask), legendRight);
  } else {
    chip("Missed it", "failed", () => grade("failed"), label(KEYS.failed));
    chip("Partly", "partly", () => grade("partly"), label(KEYS.partly));
    chip("Got it", "passed", () => grade("passed"), label(KEYS.passed));
    chip("Ask tutor", "ask", openAsk, label(KEYS.ask), legendRight);
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
  if (augmentData) {
    if (e.key === "Escape" || e.key === "Backspace") { e.preventDefault(); closeAugment(); }
    return;
  }
  if (state.phase === "select") {
    // The open focus drawer's native picker owns its own keys (Enter to choose an
    // option must not launch the deck).
    if (e.target.closest && e.target.closest(".drawer")) return;
    if (e.key === "Enter") {
      const b = legend.querySelector(".chip.primary");
      if (b && !b.disabled) { e.preventDefault(); b.click(); }
    } else if ((e.key === "Escape" || e.key === "Backspace") && selectEsc) {
      e.preventDefault(); selectEsc();
    }
    return;
  }
  // The ask overlay: Esc closes, the save-note key saves; the textarea handles
  // typing, Enter for a newline, and Shift+Enter to send.
  if (asking) {
    if (askConfirmingClose) {
      if (e.key === "Escape") { e.preventDefault(); cancelAskLeave(); }
      return;
    }
    if (e.key === "Escape") { e.preventDefault(); closeAsk(); return; }
    if (hit(e, KEYS.make_note)) { e.preventDefault(); saveAskNote(); return; }
    if (hit(e, KEYS.make_card)) { e.preventDefault(); draftCard(); return; }
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
    if ((revealed > 0 || feedback) && hit(e, KEYS.ask)) { e.preventDefault(); openAsk(); return; }
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
    if (hit(e, KEYS.ask)) { e.preventDefault(); openAsk(); return; }
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
  if (hit(e, KEYS.ask)) { e.preventDefault(); openAsk(); return; }
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
  if (walk.isOpen()) { if (walk.data().phase === "reveal") openAsk(); }
  else if (isAnswered()) openAsk();
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

// A small modal sheet (keyboard shortcuts, about). Esc or a backdrop click closes.
const sheet = document.getElementById("sheet");
const sheetPanel = document.getElementById("sheetPanel");
// The interval id of whichever share/receive live-job poll is currently
// running (never generate/import's jobPoll — those keep polling in the
// background after close so a finished job still lands a toast). Set at
// kick time, cleared here rather than left to self-clear off a stray 409.
let liveJobTimer = null;
function openSheet(html) { sheetPanel.innerHTML = html; sheet.hidden = false; }
function closeSheet() {
  // A share/receive sheet closed mid-wait must not leave the wormhole child
  // running invisibly — the job only cancels when replaced/cleared
  // server-side (see `ShareJob`'s `Drop`), so an abandoned sheet has to ask
  // for that itself.
  if (sheet.dataset.shareLive) { delete sheet.dataset.shareLive; api("/api/share/close", post({})).catch(() => {}); }
  if (sheet.dataset.receiveLive) { delete sheet.dataset.receiveLive; api("/api/receive/close", post({})).catch(() => {}); }
  if (liveJobTimer) { clearInterval(liveJobTimer); liveJobTimer = null; }
  sheet.hidden = true; sheetPanel.innerHTML = "";
}
sheet.addEventListener("click", (e) => { if (e.target === sheet) closeSheet(); });
document.addEventListener("keydown", (e) => { if (!sheet.hidden && e.key === "Escape") { e.stopPropagation(); e.preventDefault(); closeSheet(); } }, true);

// Polls a job endpoint (~700ms) into #jobLine inside the open Add sheet.
// done and error land IN the sheet while it's open; closed, error falls
// back to a toast. Script-level (not a closure inside openAdd) so Task 13's
// share/receive wiring can reuse it. The job-line element is captured once,
// here at kick time, rather than re-resolved by id on every tick — a
// re-resolve would find whatever sheet happens to be open by the time a
// background job's tick lands, and write this job's progress into it.
// `onError`, if given, runs (in addition to the rendering below) when the
// job reaches the error phase — receive uses it to clear its cancel-on-close
// flag; generate/import have none and simply omit it.
function jobPoll(path, verb, onDone, onError) {
  const line = document.getElementById("jobLine");
  const t = setInterval(() => {
    api(path).then((d) => {
      if (d.phase === "done") {
        clearInterval(t);
        api(path + "/close", post({})).catch(() => {});
        onDone(d);
      } else if (d.phase === "error") {
        clearInterval(t);
        if (onError) onError(d);
        if (line && line.isConnected) {
          line.textContent = "";
          line.appendChild(el("span", "sheet-err", d.error || (verb + " failed")));
        } else {
          notice(d.error || (verb + " failed"));
        }
      } else if (line && line.isConnected) {
        line.textContent = verb + "… " + (d.elapsed || 0) + "s";
      }
    }).catch(() => { clearInterval(t); notice(verb + " failed — the server log has details"); });
  }, 700);
  return t;
}

// Fills the Add sheet's destination <select> with Library root + every
// workspace name, from GET /api/decks. Built with DOM Option nodes (never
// innerHTML) since a workspace name is a user-chosen folder name.
function fillDestOptions(sel, d) {
  sel.innerHTML = "";
  sel.appendChild(new Option("Library root", ""));
  (d.workspaces || []).forEach((w) => sel.appendChild(new Option(w.name, w.name)));
}

function addDone(msg) { closeSheet(); notice(msg); renderSelect(); }

// The focused picker row's canonical name (deck, workspace, or folder) — the
// same `focusedEl` the depth-menu split button reads (see `renderList`),
// reused here for the kebab-menu actions that aren't rows themselves. Null
// with nothing focused (e.g. an empty catalog).
function focusedRowName() {
  const row = focusedEl;
  return (row && ((row._item && row._item.name) || (row._open && row._open.name))) || null;
}

// The Add-deck sheet: generate from a URL, import a file, or receive a
// wormhole share/zip, all landing in a chosen destination (library root or a
// workspace). Fetches the destination list before opening so the <select>
// never flashes empty.
function openAdd() {
  api("/api/decks").catch(() => ({})).then((d) => {
    openSheet(
      '<h2>Add deck</h2><div class="sheet-add">' +
      '<label>Into <select id="addDest" class="bar-filter"></select></label>' +
      '<div class="add-sec"><h3>Generate from a URL</h3>' +
      '<input id="genUrl" class="bar-filter" placeholder="https://…" autocomplete="off" spellcheck="false">' +
      '<input id="genGuide" class="bar-filter" placeholder="guidance (optional)" autocomplete="off">' +
      '<button id="genGo" class="bar-chip">Generate</button></div>' +
      '<div class="add-sec"><h3>Import a file</h3>' +
      '<input id="impFile" type="file" accept=".tsv,.txt"></div>' +
      '<div class="add-sec"><h3>Receive</h3>' +
      '<input id="rcvCode" class="bar-filter" placeholder="wormhole code" autocomplete="off" spellcheck="false">' +
      '<button id="rcvGo" class="bar-chip">Receive</button>' +
      '<input id="rcvZip" type="file" accept=".zip"></div>' +
      '<p id="jobLine"></p></div>'
    );
    fillDestOptions(document.getElementById("addDest"), d);
    const dest = () => document.getElementById("addDest").value;
    document.getElementById("genGo").addEventListener("click", () => {
      const url = document.getElementById("genUrl").value.trim();
      if (!url) return;
      const guidance = document.getElementById("genGuide").value.trim() || null;
      // Captured once, here at kick time, rather than re-resolved by id when
      // the response lands — a re-resolve could find the sheet already
      // closed (null) or a different sheet's line. See jobPoll's comment.
      const line = document.getElementById("jobLine");
      api("/api/generate", post({ url, guidance, dest: dest() || null }))
        .then((d) => {
          if (d && d.phase === "error") {
            const msg = d.error || "could not start generating";
            if (line && line.isConnected) { line.textContent = msg; } else { notice(msg); }
            return;
          }
          jobPoll("/api/generate", "generating", (r) => addDone("deck '" + r.deck + "' added"));
        })
        .catch(() => notice("could not start generating — the server log has details"));
      if (line && line.isConnected) { line.textContent = "generating… 0s"; }
    });
    document.getElementById("impFile").addEventListener("change", (e) => {
      const f = e.target.files[0];
      if (!f) return;
      const line = document.getElementById("jobLine");
      const r = new FileReader();
      r.onload = () => {
        api("/api/import", post({ name: f.name, text: r.result, dest: dest() || null }))
          .then((d2) => addDone("imported " + d2.cards + " cards into '" + d2.deck + "'"))
          .catch(() => {
            const msg = "import failed — not a valid deck, or the name is taken";
            if (line && line.isConnected) line.textContent = msg;
            else notice(msg);
          });
      };
      r.readAsText(f);
    });
    document.getElementById("rcvGo").addEventListener("click", () => {
      const code = document.getElementById("rcvCode").value.trim();
      if (!code) return;
      // Captured once, here at kick time, rather than re-resolved by id when
      // the response lands — a re-resolve could find the sheet already
      // closed (null) or a different sheet's line. See jobPoll's comment.
      const line = document.getElementById("jobLine");
      api("/api/receive", post({ code, dest: dest() || null }))
        .then((d) => {
          if (d && d.phase === "error") {
            const msg = d.error || "could not start receiving";
            if (line && line.isConnected) { line.textContent = msg; } else { notice(msg); }
            return;
          }
          sheet.dataset.receiveLive = "1"; // closeSheet() must cancel the job if abandoned
          liveJobTimer = jobPoll(
            "/api/receive", "receiving",
            (r) => { delete sheet.dataset.receiveLive; addDone("received '" + r.landed + "'"); },
            () => { delete sheet.dataset.receiveLive; }
          );
        })
        .catch(() => notice("could not start receiving — the server log has details"));
      if (line && line.isConnected) { line.textContent = "receiving… 0s"; }
    });
    document.getElementById("rcvZip").addEventListener("change", (e) => {
      const f = e.target.files[0];
      if (!f) return;
      f.arrayBuffer().then((buf) =>
        apiClient.fetch("/api/receive/zip?dest=" + encodeURIComponent(dest()), { method: "POST", body: buf })
          .then((resp) => { if (!resp.ok) throw 0; return resp.json(); })
          .then((r) => addDone("received '" + r.landed + "'"))
          .catch(() => { document.getElementById("jobLine").textContent = "could not unpack that zip"; })
      );
    });
  });
}

// The Share sheet: send the focused row (or, unfocused, the whole library)
// device-to-device via a wormhole code, or fall back to a plain zip download.
// `deck: null` in the POST body shares the served root, matching the zip
// link's bare (no `?deck=`) href — both resolve server-side the same way.
function openShare() {
  const row = focusedRowName();
  const zipHref = "/api/share/zip" + (row ? "?deck=" + encodeURIComponent(row) : "");
  openSheet(
    '<h2>Share</h2><div class="sheet-add">' +
    '<p>Share <b></b> device-to-device. Progress and personal config stay home.</p>' +
    '<button id="shareGo" class="bar-chip">Get code</button>' +
    '<p><a id="shareZip" download>or download as .zip</a></p>' +
    '<p id="jobLine"></p></div>'
  );
  sheetPanel.querySelector("b").textContent = row || "the whole library";
  document.getElementById("shareZip").href = withToken(zipHref);
  document.getElementById("shareGo").addEventListener("click", () => {
    // Captured once, here at kick time, rather than re-resolved by id when
    // the response lands or on every tick — a re-resolve could find the
    // sheet already closed (null) or a different sheet's line. See
    // jobPoll's comment for why that matters.
    const line = document.getElementById("jobLine");
    api("/api/share", post({ deck: row }))
      .then((d) => {
        if (d && d.phase === "error") {
          const msg = d.error || "could not start sharing";
          if (line && line.isConnected) { line.textContent = msg; } else { notice(msg); }
          return;
        }
        if (line && line.isConnected) { line.textContent = "staging…"; }
        sheet.dataset.shareLive = "1"; // closeSheet() must cancel the job if abandoned
        liveJobTimer = setInterval(() => {
          api("/api/share").then((d2) => {
            if (d2.phase === "code") {
              if (!line || !line.isConnected) return; // nothing to render, and not terminal
              // Rendered once and left alone — rebuilding this node every
              // tick would wipe the user's in-progress selection of the code.
              if (!line.querySelector(".share-code")) {
                line.textContent = "";
                line.appendChild(el("span", "share-code", d2.code));
                line.appendChild(document.createElement("br"));
                line.appendChild(document.createTextNode("waiting for the receiver…"));
              }
            } else if (d2.phase === "sent") {
              clearInterval(liveJobTimer);
              delete sheet.dataset.shareLive; // already closing it below — don't double-close
              api("/api/share/close", post({})).catch(() => {});
              closeSheet();
              notice("sent");
            } else if (d2.phase === "error") {
              clearInterval(liveJobTimer);
              if (line && line.isConnected) {
                line.textContent = "";
                line.appendChild(el("span", "sheet-err", d2.error || "share failed"));
              } else {
                notice(d2.error || "share failed");
              }
            }
          }).catch(() => clearInterval(liveJobTimer));
        }, 700);
      })
      .catch(() => notice("could not start sharing — the server log has details"));
  });
}
document.getElementById("mShare").addEventListener("click", () => { menu.classList.remove("open"); openShare(); });

// The Reset sheet: wipe a row's review progress, gated on typing its exact
// name back (a plain confirm dialog is too easy to reflex-click through for
// something this destructive). No focused row → nothing to reset.
function openReset() {
  const row = focusedRowName();
  if (!row) { notice("focus a deck first"); return; }
  openSheet(
    '<h2>Reset progress</h2><div class="sheet-add">' +
    '<p>Wipes all review progress for <b></b> — schedules, history, exam state. This cannot be undone.</p>' +
    '<input id="resetConfirm" class="bar-filter" placeholder="type the name to confirm" autocomplete="off" spellcheck="false">' +
    '<button id="resetGo" class="bar-chip" disabled>Reset</button><p id="jobLine"></p></div>'
  );
  sheetPanel.querySelector("b").textContent = row;
  const input = document.getElementById("resetConfirm");
  const go = document.getElementById("resetGo");
  input.addEventListener("input", () => { go.disabled = input.value !== row; });
  go.addEventListener("click", () => {
    api("/api/reset", post({ deck: row }))
      .then((d) => { closeSheet(); notice("reset " + d.cards_cleared + " card(s)"); renderSelect(); })
      .catch(() => { document.getElementById("jobLine").textContent = "reset failed — the server log has details"; });
  });
}
document.getElementById("mReset").addEventListener("click", () => { menu.classList.remove("open"); openReset(); });

// The Doctor sheet: one row per environment/backend check from /api/doctor
// (config, store, decks, backend, share, wormhole — an open set), each with a
// status glyph and, when something needs fixing, a muted remedy line.
function doctorRow(r) {
  const glyph = r.status === "ok" ? "✓" : r.status === "warn" ? "!" : r.status === "fail" ? "✗" : "?";
  const row = el("div", "doc-row doc-" + r.status);
  row.appendChild(el("span", "doc-glyph", glyph));
  const body = el("span");
  body.appendChild(el("b", null, r.name));
  body.appendChild(el("span", "doc-detail", " — " + r.detail));
  if (r.remedy) body.appendChild(el("span", "doc-remedy", r.remedy));
  row.appendChild(body);
  return row;
}
function openDoctor() {
  api("/api/doctor").then((d) => {
    openSheet('<h2>Doctor</h2><div class="sheet-doctor" id="docRows"></div>');
    const rows = document.getElementById("docRows");
    (d.rows || []).forEach((r) => rows.appendChild(doctorRow(r)));
  }).catch(() => notice("could not run the checks"));
}
document.getElementById("mDoctor").addEventListener("click", () => { menu.classList.remove("open"); openDoctor(); });

// The Pair sheet: a QR + URL for reaching this instance from another device.
// Localhost-only servers get a plain hint instead (nothing to scan).
function openPair() {
  api("/api/pair").then((d) => {
    if (!d.lan) {
      openSheet('<h2>Pair a device</h2><p class="sheet-hint"></p>');
      sheetPanel.querySelector(".sheet-hint").textContent =
        "This server is localhost-only — start alix with --lan to pair another device.";
      return;
    }
    openSheet(
      '<h2>Pair a device</h2><div class="sheet-pair">' +
      '<div class="pair-qr"></div><p class="pair-url"></p>' +
      '<p class="sheet-hint">Scan, or open the link on the other device.</p></div>'
    );
    if (d.svg) {
      // d.svg is a complete, self-contained <svg> rendered server-side by our
      // own qr::svg (same-origin, trusted, documented in docs/API.md as safe
      // to inject) — the one sanctioned innerHTML use for API data, scoped to
      // this dedicated container and nothing else concatenated into it.
      sheetPanel.querySelector(".pair-qr").innerHTML = d.svg;
    }
    sheetPanel.querySelector(".pair-url").textContent = d.url;
  }).catch(() => notice("could not fetch the pairing info"));
}
document.getElementById("mPair").addEventListener("click", () => { menu.classList.remove("open"); openPair(); });

function openShortcuts() {
  openSheet(
    '<h2>Picker shortcuts</h2><div class="sheet-keys">' +
    '<kbd>/</kbd><span>filter the list</span>' +
    '<kbd>↑ ↓</kbd><span>move</span>' +
    '<kbd>enter</kbd><span>open / start</span>' +
    '<kbd>v</kbd><span>choose a depth, then <kbd>1</kbd> <kbd>2</kbd> <kbd>3</kbd> (<kbd>c</kbd> crams)</span>' +
    '<kbd>b</kbd><span>browse the deck</span>' +
    '<kbd>x</kbd><span>take the exam</span>' +
    '<kbd>m</kbd><span>mastered decks</span>' +
    '<kbd>g / G</kbd><span>top / bottom</span>' +
    '<kbd>← →</kbd><span>step regions (in the focus drawer)</span>' +
    '<kbd>r</kbd><span>refresh the deck list</span>' +
    '<kbd>esc / ⌫</kbd><span>back</span>' +
    '</div>'
  );
}

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

document.getElementById("mShortcuts").addEventListener("click", () => { menu.classList.remove("open"); openShortcuts(); });
document.getElementById("mAdd").addEventListener("click", () => { menu.classList.remove("open"); openAdd(); });
document.getElementById("mAbout").addEventListener("click", () => {
  menu.classList.remove("open");
  api("/api/version").catch(() => ({})).then((d) => {
    const v = d && d.version ? "v" + d.version : "";
    openSheet(
      '<h2>About</h2><div class="sheet-about">' +
      '<p class="about-name">alix <b>' + v + '</b></p>' +
      '<p class="about-tag">Spaced repetition with an AI exam that checks understanding. Early and changing fast.</p>' +
      '<p><a href="https://alix.study" target="_blank" rel="noopener">alix.study</a></p>' +
      '<p class="about-support">Free and open source. Telling someone who studies is the best support. ' +
      '<a href="https://github.com/sponsors/Alex6323" target="_blank" rel="noopener">Sponsor</a></p>' +
      '</div>'
    );
  });
});

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
    if (ai) askInfo = ai;
    if (bk) BK = bk;
    PK = Object.assign({
      up: [{ k: "k", ctrl: false }], down: [{ k: "j", ctrl: false }],
      open: [{ k: "l", ctrl: false }], back: [{ k: "h", ctrl: false }],
      filter: [{ k: "/", ctrl: false }, { k: "f", ctrl: true }], mastered: [{ k: "m", ctrl: false }],
      depth: [{ k: "v", ctrl: false }], recognize: [{ k: "1", ctrl: false }],
      recall: [{ k: "2", ctrl: false }], reconstruct: [{ k: "3", ctrl: false }],
      cram: [{ k: "c", ctrl: false }],
    }, pk || {});
    document.querySelector("#mRemove .mk").textContent = label(KEYS.remove);
    document.getElementById("mAskKey").textContent = label(KEYS.ask);
    return load();
  }).catch(() => setTimeout(boot, 500));
}
boot();
