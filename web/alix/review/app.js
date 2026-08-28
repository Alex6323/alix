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
// The configured backend's display name ("Claude", "Copilot", …) and the
// shared "X is working…" progress line the exam and augment overlays show:
// one place, so no surface can drift back to a hardcoded backend.
function workingText(s) {
  if (s < 2) return `${tutor.backendName()} is working…`;
  if (s < 90) return `${tutor.backendName()} is working… ${s}s`;
  return `${tutor.backendName()} is working… ${Math.floor(s / 60)}m ${s % 60}s (this can take a couple of minutes)`;
}
let study = null;

const apiClient = createApiClient({
  fetchImpl: window.fetch.bind(window),
  sessionStorage,
  onUnauthorized: showTokenGate,
  revision: () => study?.state()?.study_revision,
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

study = createStudy({
  api,
  post,
  storage: localStorage,
  lastDeck: () => sessionStorage.getItem("alix.lastDeck"),
  openAugment: (deck) => augment.open(deck),
  model: { create: createModel, applyStudyState, enterPicker, currentScreen },
  rerender: render,
  walkData: () => walk.data(),
  replaceWalk: (next) => walk.replace(next),
  openTutor: () => tutor.show(),
  startExam: (deck) => exam.start(deck),
  closeMenu: () => menu.classList.remove("open"),
  notice,
  timers: {
    setInterval: window.setInterval.bind(window),
    clearInterval: window.clearInterval.bind(window),
    setTimeout: window.setTimeout.bind(window),
    requestAnimationFrame: window.requestAnimationFrame.bind(window),
  },
  ui: {
    appendChecklist,
    appendChoiceOptions,
    appendQuote,
    appendContext,
    appendKeypointList,
    appendReveal,
    appendRuns,
    appendTable,
    chip,
    clearLegendSides,
    computedStyle: window.getComputedStyle.bind(window),
    diagramImage,
    maskedImage,
    crumbStrip,
    deckEl,
    document,
    drawButton: document.getElementById("mDraw"),
    drawState: document.getElementById("mDrawState"),
    el,
    frontEl,
    headerBreadcrumb,
    histEl,
    hit,
    label,
    legend,
    legendLeft,
    legendRight,
    menuWrap,
    overflowHints,
    renderNote,
    scoreEl,
    setMenuContext,
    stage,
    window,
  },
});

const picker = createPicker({
  api,
  post,
  sessionStorage,
  currentState: study.state,
  isBrowsing: study.isBrowsing,
  examIsOpen: () => exam.isOpen(),
  augmentIsOpen: () => augment.isOpen(),
  walkIsOpen: () => walk.isOpen(),
  tutorIsOpen: () => tutor.isOpen(),
  applyStudy: study.apply,
  openWalk: (next) => walk.open(next),
  openBrowse: study.openBrowse,
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
  applyStudy: study.apply,
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
    state: study.state,
    replaceState: study.replaceState,
    load: study.load,
  },
  ui: {
    appendContext,
    appendQuote,
    appendRuns,
    appendRunsOrText,
    appendTable,
    chip,
    diagramImage,
    document,
    maskedImage,
    el,
    hit,
    keys: study.keys,
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
  applyStudy: study.apply,
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
    buildCardShell: study.buildCardShell,
    chip,
    deckEl,
    el,
    headerBreadcrumb,
    histEl,
    hit,
    keys: () => study.keys(),
    label,
    legend,
    legendLeft,
    legendRight,
    menuWrap,
    renderSourceExcerpt: study.renderSourceExcerpt,
    requestAnimationFrame: window.requestAnimationFrame.bind(window),
    scoreEl,
    setMenuContext,
    setTimeout: window.setTimeout.bind(window),
    sourceTerms: study.sourceTerms,
    stage,
    updateFade: study.updateFade,
  },
});

const augment = createAugment({
  api,
  post,
  rememberLaunch: picker.rememberLaunch,
  rerender: render,
  applyStudy: study.apply,
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

function render() {
  study.prepareRender();
  stage.innerHTML = "";
  crumbStrip.innerHTML = "";
  legend.innerHTML = "";
  clearLegendSides();
  menu.classList.remove("open");

  if (exam.isOpen()) { exam.render(); return; }
  if (augment.isOpen()) { augment.render(); return; }
  const screen = study.screen();
  if (screen === "walk") { walk.render(); return; }
  if (screen === "browse") { study.render(); return; }
  if (screen === "picker") { picker.render(); return; }
  if (tutor.isOpen()) { study.prepareSurface(); tutor.render(); return; }
  study.render();
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

document.addEventListener("keydown", (event) => {
  if (event.altKey || event.metaKey) return;
  if (walk.isOpen()) { walk.handleKey(event); return; }
  if (!study.state()) return;
  if (exam.isOpen()) { exam.handleKey(event); return; }
  if (study.isBrowsing()) { study.handleKey(event); return; }
  if (augment.isOpen()) { augment.handleKey(event); return; }
  if (study.state().phase === "select") { picker.handleKey(event); return; }
  if (tutor.isOpen()) { tutor.handleKey(event); return; }
  study.handleKey(event);
});

document.getElementById("kebab").addEventListener("click", (e) => { e.stopPropagation(); study.syncDrawMenu(); menu.classList.toggle("open"); });
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
  else if (study.isAnswered()) tutor.show();
});
document.getElementById("mRemove").addEventListener("click", () => { menu.classList.remove("open"); study.remove(); });
const mDraw = document.getElementById("mDraw");
study.syncDrawMenu();
mDraw.addEventListener("click", study.toggleDraw);
document.addEventListener("click", () => menu.classList.remove("open"));


// Show the right menu items for the current screen (picker vs review vs walk).
function setMenuContext(ctx) {
  document.querySelectorAll("#menu .m-picker").forEach((b) => { b.style.display = ctx === "picker" ? "" : "none"; });
  // Ask Tutor is the one .m-review item that also makes sense mid-walk; the
  // rest (Remove card, Promote) are per-deck-card actions a trace checkpoint
  // doesn't have, so they get their own narrower checks below.
  document.querySelectorAll("#menu .m-review").forEach((b) => { b.style.display = (ctx === "review" || ctx === "walk") ? "" : "none"; });
  document.getElementById("mRemove").style.display = ctx === "review" ? "" : "none";
  // (a remediation card) — narrower than the other .m-review items, so it
  // gets its own check on top of the context toggle.
  barNav.style.display = ctx === "picker" ? "" : "none";
}

document.getElementById("mShortcuts").addEventListener("click", () => { menu.classList.remove("open"); sheets.openShortcuts(); });
document.getElementById("mAdd").addEventListener("click", () => { menu.classList.remove("open"); sheets.openAdd(); });
document.getElementById("mShare").addEventListener("click", () => { menu.classList.remove("open"); sheets.openShare(); });
document.getElementById("mDelete").addEventListener("click", () => { menu.classList.remove("open"); sheets.openLibraryRemoval(); });
document.getElementById("mReset").addEventListener("click", () => { menu.classList.remove("open"); sheets.openReset(); });
document.getElementById("mDoctor").addEventListener("click", () => { menu.classList.remove("open"); sheets.openDoctor(); });
document.getElementById("mPair").addEventListener("click", () => { menu.classList.remove("open"); sheets.openPair(); });
document.getElementById("mAbout").addEventListener("click", () => { menu.classList.remove("open"); sheets.openAbout(); });

// Load the key bindings first, then the session, and retry on a transient
// failure. A just-started server, or the browser reusing a dead keep-alive
// connection from a killed one, can fail the first request; the picker-keys
// fallback keeps the Vim defaults when that one request fails. Without the
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
    study.setKeys(k);
    tutor.setInfo(ai);
    study.setBrowseKeys(bk);
    picker.setKeys(pk);
    document.querySelector("#mRemove .mk").textContent = label(k.remove);
    document.getElementById("mAskKey").textContent = label(k.ask);
    return study.load();
  }).catch(() => setTimeout(boot, 500));
}
boot();
