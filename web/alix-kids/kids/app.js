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
  for (const name of theme.names()) {
    const t = theme.palette(name);
    const b = el("button", "swatch");
    b.type = "button";
    b.dataset.theme = name;
    b.title = name;
    b.setAttribute("aria-label", name);
    b.style.background = "linear-gradient(168deg, " + t.bgTop + ", " + t.bgBot + ")";
    const dot = el("span", "swatch-dot");
    dot.style.background = t.accent;
    b.appendChild(dot);
    b.addEventListener("click", () => { theme.set(name); updateSwatchState(); closeMenu(); });
    host.appendChild(b);
  }
  updateSwatchState();
}
function updateSwatchState() {
  const list = document.querySelectorAll(".swatch");
  for (const s of list) s.setAttribute("aria-pressed", String(s.dataset.theme === theme.current()));
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
