"use strict";

captureKidsPairingToken({ location, history, sessionStorage });

const theme = createKidsTheme({
  storage: localStorage,
  rootStyle: document.documentElement.style,
});

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

const settings = createKidsSettings({
  theme,
  ui: {
    backdrop: menuBackdrop,
    button: menuBtn,
    document,
    el,
    host: document.getElementById("themes"),
    popup: menuPop,
  },
});
menuBtn.addEventListener("click", settings.toggle);
menuBackdrop.addEventListener("click", settings.close);
document.addEventListener("keydown", settings.handleKey);

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
let tutor;

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
  openTutor: () => tutor.open(),
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

tutor = createKidsTutor({
  api,
  post,
  resyncStudy: study.resync,
  timers: { setInterval, clearInterval },
  ui: {
    mascot: mascotEl,
    input: askInput,
    log: askLog,
    overlay: askOverlay,
    sendButton: askSendBtn,
    el,
  },
});

askSendBtn.addEventListener("click", tutor.send);
askInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    event.preventDefault();
    tutor.send();
  }
});
askCloseBtn.addEventListener("click", tutor.close);
askOverlay.addEventListener("click", (event) => {
  if (event.target === askOverlay) tutor.close();
});
document.addEventListener("keydown", (event) => {
  const state = study.state();
  tutor.handleKey(event, !!(state && state.kind === "review" && state.card));
});

// ── Render dispatcher ─────────────────────────────────────────────────────
function render() {
  theme.apply();
  stage.innerHTML = "";
  actionbar.innerHTML = "";
  if (study.isOpen()) study.render();
  else picker.render();
  pokeFades();
}
function updateFades() {
  const hints = kidsOverflowHints(stage);
  fadeTop.classList.toggle("show", hints.showTop);
  fadeBot.classList.toggle("show", hints.showBottom);
}
// Re-check after layout settles (fonts, images, screen swaps).
function pokeFades() {
  requestAnimationFrame(updateFades);
  [40, 160, 360].forEach((ms) => setTimeout(updateFades, ms));
}
stage.addEventListener("scroll", updateFades, { passive: true });
if (window.ResizeObserver) { new ResizeObserver(updateFades).observe(stage); }
window.addEventListener("resize", updateFades);

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

settings.build();
render();       // paints the splash immediately
picker.load();  // then fills the boxes
