"use strict";

captureKidsPairingToken({ location, history, sessionStorage });

// ── View state only (the engine behind /api/* owns everything else) ───────
let askOpen = false;     // Ask-Alix overlay view state
let askData = { transcript: [], thinking: false, status: null, error: null };
let askPoll = null;      // GET /api/ask poll interval id while askData.thinking

// ── Themes (accent-only; brand orange is never themed) ────────────────────
const THEMES = {
  Sunrise: { bgTop: "#fff5ec", bgBot: "#ffe7d4", accent: "#ff8a3d", sh: "#e0702a" },
  Ocean:   { bgTop: "#eafaf7", bgBot: "#cdeeff", accent: "#0fa8b4", sh: "#0b7d86" },
  Berry:   { bgTop: "#fdeefb", bgBot: "#f1dbff", accent: "#c04bd0", sh: "#9c34ab" },
};
let theme = loadTheme();

function loadTheme() {
  try {
    const t = localStorage.getItem("alix-kids-theme");
    return THEMES[t] ? t : "Sunrise";
  } catch (e) { return "Sunrise"; }
}
function applyTheme() {
  const t = THEMES[theme] || THEMES.Sunrise;
  const r = document.documentElement.style;
  r.setProperty("--bg-top", t.bgTop);
  r.setProperty("--bg-bot", t.bgBot);
  r.setProperty("--bg", "linear-gradient(168deg, " + t.bgTop + " 0%, " + t.bgBot + " 100%)");
  r.setProperty("--accent", t.accent);
  r.setProperty("--accent-sh", t.sh);
}
function setTheme(name) {
  if (!THEMES[name]) return;
  theme = name;
  try { localStorage.setItem("alix-kids-theme", name); } catch (e) { /* ignore */ }
  applyTheme();
  updateSwatchState();
}

// ── DOM handles ───────────────────────────────────────────────────────────
const stage = document.getElementById("stage");
const actionbar = document.getElementById("actionbar");
const fadeTop = document.getElementById("fadeTop");
const fadeBot = document.getElementById("fadeBot");
const menuBtn = document.getElementById("menuBtn");
const menuPop = document.getElementById("menuPop");
const menuBackdrop = document.getElementById("menuBackdrop");
const askOverlay = document.getElementById("askOverlay");
const askLog = document.getElementById("askLog");
const askInput = document.getElementById("askInput");
const askSendBtn = document.getElementById("askSendBtn");
const askCloseBtn = document.getElementById("askCloseBtn");

const { appendChecklist, appendRuns, contextLine, el, frontPrompt, mascot: mascotEl } = createKidsDom({ document });

const errorReporter = createKidsErrorReporter({
  console,
  timers: { setTimeout, clearTimeout },
  ui: { oops: document.getElementById("oops") },
});
function showOops(detail) { errorReporter.show(detail); }
window.addEventListener("unhandledrejection", errorReporter.handleUnhandledRejection);
window.addEventListener("error", errorReporter.handleError);

let picker;
let study;

const kidsApi = createKidsApiClient({
  fetchImpl: window.fetch.bind(window),
  sessionStorage,
  onUnauthorized: showGate,
  revision: () => study && study.revision(),
});
const api = kidsApi.request;
const post = kidsApi.postOptions;

study = createKidsStudy({
  api,
  post,
  model: {
    create: createKidsStudyModel,
    apply: applyKidsStudyState,
    clear: clearKidsStudyState,
    choose: chooseKidsAnswer,
    reveal: revealKidsAnswer,
    backCount: kidsBackCount,
    choiceMode: kidsChoiceMode,
    revealDone: kidsRevealDone,
    screen: kidsStudyScreen,
  },
  rerender: render,
  openTutor,
  openPicker: () => picker.home(),
  refreshPicker: () => picker.load(),
  reportError: showOops,
  ui: { actionbar, appendChecklist, appendRuns, contextLine, document, el, frontPrompt, mascot: mascotEl, stage },
});

picker = createKidsPicker({
  api,
  post,
  openStudy: study.apply,
  rerender: render,
  isVisible: () => !study.isOpen(),
  ui: { actionbar, document, el, mascot: mascotEl, stage },
});


// ── render() dispatches on `screen`; Tasks 5-7 flesh out their branches ───
function render() {
  applyTheme();
  stage.innerHTML = "";
  actionbar.innerHTML = "";
  if (study.isOpen()) study.render();
  else picker.render();
  pokeFades();
}


// ── Ask-Alix tutor overlay ─────────────────────────────────────────────────
// A card-scoped chat, mirroring review.html's ask wiring (openAsk / sendAsk /
// startAskPoll / syncAsk) but as a static modal shell -- like the settings menu
// or the pairing gate -- instead of a stage takeover, since kids.html never
// tears down the review screen behind it. The client sends only {question};
// the server derives the current card as context and applies the kid-safe
// system prompt (Task 2). No note-saving / model display in kids v1.
document.getElementById("askMascotSlot").appendChild(mascotEl("mascot-sm"));

function openTutor() {
  askOpen = true;
  askOverlay.hidden = false;
  syncAsk();
  api("/api/ask").then((d) => { askData = d; if (askOpen) syncAsk(); if (d.thinking) startAskPoll(); }); // prior transcript
}
function closeTutor() {
  askOpen = false;
  stopAskPoll();
  askOverlay.hidden = true;
}
function sendAskMsg() {
  const q = askInput.value.trim();
  if (!q || askData.thinking) return;
  askInput.value = "";
  api("/api/ask", post({ question: q })).then((d) => { askData = d; if (askOpen) syncAsk(); startAskPoll(); }).catch(study.resync);
}
function startAskPoll() {
  stopAskPoll();
  askPoll = setInterval(() => {
    api("/api/ask").then((d) => { askData = d; if (!d.thinking) stopAskPoll(); if (askOpen) syncAsk(); });
  }, 400);
}
function stopAskPoll() { if (askPoll) { clearInterval(askPoll); askPoll = null; } }

// (Re)fill the bubble log + input/send disabled state from askData. A
// greeting bubble stands in until the first exchange; a raw askData.error
// never reaches the child -- just a warm, generic fallback line.
function syncAsk() {
  askLog.innerHTML = "";
  if (!askData.transcript.length && !askData.thinking && !askData.error) {
    askLog.appendChild(el("div", "ask-bubble ask-bubble-a", "Hi! I'm Alix 🦊 Ask me anything about this card and I'll explain it in a fun way."));
  }
  for (const ex of askData.transcript) {
    askLog.appendChild(el("div", "ask-bubble ask-bubble-q", ex.q));
    askLog.appendChild(el("div", "ask-bubble ask-bubble-a", ex.a));
  }
  if (askData.thinking) askLog.appendChild(el("div", "ask-bubble ask-bubble-a ask-bubble-think", "Alix is thinking… 🤔"));
  if (askData.error) askLog.appendChild(el("div", "ask-bubble ask-bubble-a", "Oops, I couldn't think just now. Try asking again! 🦊"));
  askLog.scrollTop = askLog.scrollHeight;

  askInput.disabled = askData.thinking;
  askSendBtn.disabled = askData.thinking;
  if (!askData.thinking && askOpen) askInput.focus();
}

askSendBtn.addEventListener("click", sendAskMsg);
askInput.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); sendAskMsg(); } });
askCloseBtn.addEventListener("click", closeTutor);
askOverlay.addEventListener("click", (e) => { if (e.target === askOverlay) closeTutor(); }); // tap-out
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && askOpen) { closeTutor(); return; }
  const state = study.state();
  if (e.key === "?" && !askOpen && state && state.kind === "review" && state.card) openTutor();
});

// A trace deck (a WalkDto) can't be walked in kids v1 -- bow out gently instead
// of mis-rendering it. Doubles as the empty/lost-session fallback.
function updateFades() {
  const s = stage;
  const up = s.scrollTop > 4;
  const down = (s.scrollHeight - s.clientHeight - s.scrollTop) > 4;
  fadeTop.classList.toggle("show", up);
  fadeBot.classList.toggle("show", down);
}
// Re-check after layout settles (fonts, images, screen swaps).
function pokeFades() {
  requestAnimationFrame(updateFades);
  [40, 160, 360].forEach((ms) => setTimeout(updateFades, ms));
}
stage.addEventListener("scroll", updateFades, { passive: true });
if (window.ResizeObserver) { new ResizeObserver(updateFades).observe(stage); }
window.addEventListener("resize", updateFades);

// ── Settings menu + theme swatches ────────────────────────────────────────
function buildThemeSwatches() {
  const host = document.getElementById("themes");
  host.innerHTML = "";
  for (const name of Object.keys(THEMES)) {
    const t = THEMES[name];
    const b = el("button", "swatch");
    b.type = "button";
    b.dataset.theme = name;
    b.title = name;
    b.setAttribute("aria-label", name);
    b.style.background = "linear-gradient(168deg, " + t.bgTop + ", " + t.bgBot + ")";
    const dot = el("span", "swatch-dot");
    dot.style.background = t.accent;
    b.appendChild(dot);
    b.addEventListener("click", () => { setTheme(name); closeMenu(); });
    host.appendChild(b);
  }
  updateSwatchState();
}
function updateSwatchState() {
  const list = document.querySelectorAll(".swatch");
  for (const s of list) s.setAttribute("aria-pressed", String(s.dataset.theme === theme));
}
function openMenu() {
  menuPop.hidden = false;
  menuBackdrop.hidden = false;
  menuBtn.setAttribute("aria-expanded", "true");
  updateSwatchState();
}
function closeMenu() {
  menuPop.hidden = true;
  menuBackdrop.hidden = true;
  menuBtn.setAttribute("aria-expanded", "false");
}
menuBtn.addEventListener("click", () => { menuPop.hidden ? openMenu() : closeMenu(); });
menuBackdrop.addEventListener("click", closeMenu);
document.addEventListener("keydown", (e) => { if (e.key === "Escape") closeMenu(); });

// ── Token gate (a 401 under `alix --lan`) ─────────────────────────────────
function showGate() {
  const g = document.getElementById("tokengate");
  if (!g || !g.hidden) return;
  g.hidden = false;
  document.getElementById("gateInput").focus();
}
function gateConnect() {
  const t = document.getElementById("gateInput").value.trim();
  if (!t) return;
  try { sessionStorage.setItem("alix.token", t); } catch (e) { /* ignore */ }
  location.reload();
}
document.getElementById("gateBtn").addEventListener("click", gateConnect);
document.getElementById("oops").addEventListener("click", (e) => { e.currentTarget.hidden = true; });
document.getElementById("gateInput").addEventListener("keydown", (e) => { if (e.key === "Enter") gateConnect(); });

// ── Load ──────────────────────────────────────────────────────────────────
// Home reads the box catalog once (no live counter -- a receding queue is a
// false finish line). refresh() re-reads it, e.g. on returning from a session.

buildThemeSwatches();
render();       // paints the splash immediately
picker.load();  // then fills the boxes
