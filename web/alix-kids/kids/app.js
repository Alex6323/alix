"use strict";

captureKidsPairingToken({ location, history, sessionStorage });

// A rejected review action (a stale revision after a lost reply, or a
// transport hiccup) re-pulls the state calmly instead of the full oops
// screen: the next tap then carries a fresh echo.
function resyncState() {
  api("/api/state").then(applyState).catch((e) => showOops(e));
}

// ── View state only (the engine behind /api/* owns everything else) ───────
let screen = "home";     // "home" | "box" | "review" | "done"
let currentBox = null;   // the opened workspace DeckItemDto (the box)
let currentDeck = null;  // the deck (MemberDto) picked inside that box, if any
let selectError = false; // a /api/select call failed -- show a gentle notice
let deckList = null;     // last DeckListDto (null until the first load resolves)
let loadError = false;   // the /api/decks fetch failed
let studyModel = createKidsStudyModel();
let { state, revealed, chosen } = studyModel;
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

const kidsApi = createKidsApiClient({
  fetchImpl: window.fetch.bind(window),
  sessionStorage,
  onUnauthorized: showGate,
  revision: () => state && state.study_revision,
});
const api = kidsApi.request;
const post = kidsApi.postOptions;

// Mirrors the adult client: `save_error` is stateful, so the banner shows
// exactly as long as the server keeps reporting it. Raw error on the tooltip.
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
  a.textContent = "Uh oh, your progress isn't saving. Ask a grown-up to help!";
}

// ── render() dispatches on `screen`; Tasks 5-7 flesh out their branches ───
function render() {
  applyTheme();
  stage.innerHTML = "";
  actionbar.innerHTML = "";
  syncSaveAlert();
  if (screen === "home") renderHome();
  else if (screen === "box") renderBox();
  else if (screen === "review") renderReview();
  else if (screen === "done") renderDone();
  pokeFades();
}

function renderHome() {
  // Before the first /api/decks resolves, show the brand-mark splash.
  if (deckList == null && !loadError) {
    const splash = el("div", "splash");
    const logo = document.createElement("alix-logo");
    logo.setAttribute("loop", "");
    logo.setAttribute("height", "64");
    logo.setAttribute("color", "#ff8a3d");
    splash.appendChild(logo);
    splash.appendChild(el("div", "splash-label", "Getting your boxes ready…"));
    stage.appendChild(splash);
    return;
  }

  const home = el("div", "home");

  const greet = el("div", "greet");
  greet.appendChild(mascotEl());
  const gt = el("div");
  gt.appendChild(el("div", "greet-title", "Hi! What do you want to practise?"));
  gt.appendChild(el("div", "greet-sub", "Pick a box and let's go 🌟"));
  greet.appendChild(gt);
  home.appendChild(greet);

  if (loadError) {
    home.appendChild(el("div", "empty", "Hmm, I couldn't find your boxes just now. Try again in a moment 🌱"));
    stage.appendChild(home);
    return;
  }

  const boxes = (deckList && deckList.workspaces) || [];
  if (!boxes.length) {
    home.appendChild(el("div", "empty", "No boxes yet. Ask a grown-up to add some 🌱"));
    stage.appendChild(home);
    return;
  }

  const grid = el("div", "box-grid");
  for (const b of boxes) grid.appendChild(boxCard(b));
  home.appendChild(grid);
  stage.appendChild(home);
}

// One workspace = one box. Fields used: `icon` (a /img/<key> URL, else a
// friendly default emoji), `label` (title), and `reviewable` (a soft, honest
// readiness line -- never a fabricated "N ready" count).
function boxCard(b) {
  const card = el("button", "box");
  card.type = "button";
  card.appendChild(iconEl(b, "box-icon", "box-emoji"));
  card.appendChild(el("div", "box-name", b.label || b.name || "Box"));
  card.appendChild(el("div", "box-ready", b.reviewable ? "ready to practise" : "all caught up 🌱"));
  card.addEventListener("click", () => openBox(b));
  return card;
}

// A workspace's emblem when it has one, else a friendly default emoji. `/img`
// URLs are unauthenticated by design, so a plain <img src> is enough.
function iconEl(item, imgCls, emojiCls) {
  if (item && item.icon) {
    const img = document.createElement("img");
    img.className = imgCls;
    img.src = item.icon;
    img.alt = "";
    return img;
  }
  return el("div", emojiCls, "📚");
}

function openBox(b) { currentBox = b; currentDeck = null; selectError = false; screen = "box"; render(); }
function goHome() { screen = "home"; currentBox = null; currentDeck = null; selectError = false; render(); refresh(); }
function openDeck(m) { currentDeck = m; selectError = false; render(); }
function backToBox() { currentDeck = null; selectError = false; render(); }

// The box screen, in two steps: pick a deck, then pick how to practise it.
//
// A review session is exactly ONE deck file -- the engine rejects a whole
// workspace ("`…/animals` is a folder -- pick a deck inside it", `build_review`
// in src/cli/launch.rs). So the box is an organizing layer, and the *decks*
// are the launch controls; the depth choices belong to the deck a kid picked.
function renderBox() {
  if (currentDeck) { renderDeckLaunch(); return; }

  const b = currentBox || {};
  const back = el("button", "ghost-btn", "← Home");
  back.type = "button";
  back.addEventListener("click", goHome);
  actionbar.appendChild(back);

  const wrap = el("div", "box-detail");

  const hero = el("div", "box-hero");
  hero.appendChild(iconEl(b, "hero-icon", "hero-emoji"));
  const heroText = el("div");
  heroText.appendChild(el("div", "hero-title", b.label || b.name || "Box"));
  if (b.description) heroText.appendChild(el("div", "soft", b.description));
  hero.appendChild(heroText);
  wrap.appendChild(hero);

  const members = Array.isArray(b.members) ? b.members : [];
  if (members.length) {
    wrap.appendChild(el("div", "section-label", "Pick a deck"));
    const list = el("div", "deck-list");
    for (const m of members) list.appendChild(deckRow(m));
    wrap.appendChild(list);
  } else {
    wrap.appendChild(el("div", "soft", "This box has no decks yet."));
  }
  stage.appendChild(wrap);
}

// A deck is picked: show its two depth choices ("how do you want to practise?"),
// each gated on that deck's own honest per-depth due-ness.
function renderDeckLaunch() {
  const m = currentDeck || {};
  const b = currentBox || {};

  const back = el("button", "ghost-btn", "← " + (b.label || b.name || "Box"));
  back.type = "button";
  back.addEventListener("click", backToBox);
  actionbar.appendChild(back);

  // Deck, question and the two answers are one decision -- centre them together
  // in the stage rather than stranding the buttons in the corner of the bar.
  const wrap = el("div", "launch");
  wrap.appendChild(iconEl(b, "launch-icon", "launch-emoji"));
  wrap.appendChild(el("div", "launch-title", m.label || m.name || "Deck"));
  const sub = el("div", "deck-sub");
  sub.appendChild(masteryEl(m));
  wrap.appendChild(sub);
  wrap.appendChild(el("div", "soft", "How do you want to practise?"));

  const choices = el("div", "depth-choices launch-choices");
  choices.appendChild(depthBtn("👆 Tap the answer", "recognize", m.reviewable_recognize, m.name));
  choices.appendChild(depthBtn("🗣️ Say it yourself", "recall", m.reviewable_recall, m.name));
  wrap.appendChild(choices);

  if (selectError) wrap.appendChild(el("div", "select-error", "Hmm, that didn't start. Want to try again? 🌱"));
  stage.appendChild(wrap);
}

// One deck: tap it to choose how to practise. Shows its ⭐ mastery (or "New")
// plus a chevron, so it reads as the tappable thing it is.
function deckRow(m) {
  const row = el("button", "deck-row");
  row.type = "button";
  row.appendChild(el("div", "deck-label", m.label || m.name || "Deck"));
  const right = el("div", "deck-right");
  right.appendChild(masteryEl(m));
  right.appendChild(el("span", "deck-go", "›"));
  row.appendChild(right);
  row.addEventListener("click", () => openDeck(m));
  return row;
}

// No badge yet (or a never-seen deck) reads as "New", not zero stars.
// Otherwise one ⭐ per badged depth (recognize→1, recall→2, reconstruct→3);
// a lapsed badge (`badge_dotted`) hollows out just the highest star.
function masteryEl(m) {
  if (!m.badge_depth || m.state === "new") return el("span", "pill-new", "New");
  const n = m.badge_depth === "reconstruct" ? 3 : m.badge_depth === "recall" ? 2 : 1;
  let stars = "";
  for (let i = 0; i < n; i++) stars += (m.badge_dotted && i === n - 1) ? "☆" : "⭐";
  return el("span", "deck-mastery", stars);
}

// A depth-start choice for the picked deck, gated on that deck's honest
// per-depth due-ness. Both choices always render -- a caught-up one just can't
// be tapped, so a kid never lands in an empty session.
function depthBtn(label, depth, ready, targetName) {
  const btn = el("button", "depth-btn" + (ready ? "" : " caught-up"));
  btn.type = "button";
  btn.appendChild(el("span", null, label));
  if (ready) {
    btn.addEventListener("click", () => startSelect(targetName, depth));
  } else {
    btn.disabled = true;
    btn.appendChild(el("span", "depth-btn-note", "all caught up here 🌱"));
  }
  return btn;
}

// POST /api/select for ONE deck at the chosen depth and land on the review
// screen. Mirrors how review.html's `select()` holds the session in a
// module-level `state`; every review action funnels back through applyState.
// A rejected select (a 400 carries no JSON body, so `api` throws) surfaces a
// gentle notice instead of leaving a kid tapping a button that does nothing.
function startSelect(name, depth) {
  selectError = false;
  api("/api/select", post({ deck: name, depth }))
    .then(applyState)
    .catch(() => { selectError = true; render(); });
}

// ── The review loop ───────────────────────────────────────────────────────
// Every review action (/api/select, /api/grade, /api/acquire, /api/deselect)
// returns the NEXT StateDto -- apply it, reset the per-card view state, and route.
// `/api/select` and each action can also return a WalkDto (a trace deck); kids
// v1 handles only the review StateDto, so we branch on `kind` and route a
// non-review payload to a gentle "not ready" screen (rendered by renderReview).
function applyState(s) {
  syncStudyModel(applyKidsStudyState(studyModel, s));
  screen = kidsStudyScreen(studyModel);
  render();
}

function syncStudyModel(next) {
  studyModel = next;
  ({ state, revealed, chosen } = studyModel);
}
function backCount() { return kidsBackCount(studyModel); }
function isChoiceMode() { return kidsChoiceMode(studyModel); }
// Has the answer been fully revealed (so the mascot "why" + rate bar show)?
function revealDone() {
  return kidsRevealDone(studyModel);
}

// The heart of the app. Branches on kind (trace → not-ready), then on
// state.mode: choice → tap-the-answer, line → reveal-next, everything else →
// fill-in-the-blank. The persistent Home / Ask Alix bar renders for every card.
function renderReview() {
  // Kids handle only the review StateDto; a trace deck resolves to a WalkDto.
  if (!state || state.kind !== "review" || !state.card) { renderNotReady(); return; }

  const card = state.card;
  const acquire = !!state.acquire;
  const choiceMode = isChoiceMode();
  const lineMode = state.mode === "line";
  // A never-seen card is ATTEMPTED like any other -- attempt-first, as the
  // engine intends (it ships `choices` on acquire cards too, and /api/choose
  // answers them). Only the bar differs: one ungraded "Got it! Next" instead of
  // a rate. Forcing `done` here would skip the attempt entirely and make the
  // depth the kid just chose ("Tap the answer" / "Say it yourself") meaningless
  // for the whole first pass through a new deck.
  const done = revealDone();

  const inner = el("div", "rev-stage-inner");
  const cardEl = el("div", "rev-card");

  cardEl.appendChild(el("div", "rev-eyebrow", eyebrowFor(state, acquire)));
  appendImages(cardEl, card.images);
  cardEl.appendChild(frontPrompt(card));
  for (let i = 0; i < (card.context || []).length; i++) {
    cardEl.appendChild(contextLine(card.context[i], card.context_runs && card.context_runs[i]));
  }

  if (choiceMode) {
    cardEl.appendChild(renderOptions());
  } else if (lineMode) {
    cardEl.appendChild(renderLines(card));
  } else {
    // Fill-in-the-blank: a blank before reveal, the green answer after.
    cardEl.appendChild(done ? answerFill(card) : blankEl());
  }
  if (done) appendImages(cardEl, card.images_back);

  // Reserve the why-slot whenever the card has a note, so filling it on reveal
  // doesn't resize the card (the shell must not jump).
  if ((card.note && card.note.length > 0) || (state.keypoints && state.keypoints.length > 0)) {
    const slot = el("div", "rev-why-slot");
    if (done) renderWhy(slot, card);
    cardEl.appendChild(slot);
  }

  inner.appendChild(cardEl);
  stage.appendChild(inner);

  renderReviewBar(done, acquire, lineMode, choiceMode);
}

function eyebrowFor(s, acquire) {
  if (acquire) return "Here's a new one! ✨";
  if (isChoiceMode()) return "Tap the answer 👆";
  if (s.mode === "line") return "Line by line 📖";
  return "Fill in the blank ✏️";
}

// Each side's images render as ordered blocks; `im` is a `{ src, alt }` from
// the card's `images` / `images_back` list.
function appendImages(parent, images) {
  for (const im of (images || [])) parent.appendChild(cardImg(im));
}

function cardImg(im) {
  const img = document.createElement("img");
  img.className = "rev-img";
  img.src = im.src;
  img.alt = im.alt || "";
  return img;
}

// The green-filled answer. A multi-line answer keeps its lines -- joining them
// into one run-on string would turn an ordered sequence ("Egg / Caterpillar /
// Chrysalis / Butterfly") into nonsense.
function answerFill(card) {
  const a = el("div", "rev-answer");
  const lines = card.back || [];
  if (lines.length > 1) {
    const stack = el("div", "rev-answer-stack");
    appendBackLines(stack, card, lines.length, "span", "rev-answer-fill");
    a.appendChild(stack);
  } else {
    const answer = el("span", "rev-answer-fill");
    if (card.back_runs && card.back_runs[0]) appendRuns(answer, card.back_runs[0]);
    else answer.textContent = lines[0] || "";
    a.appendChild(answer);
  }
  return a;
}
// The underlined blank shown before a fill-in-the-blank card is revealed.
function blankEl() {
  const a = el("div", "rev-answer");
  a.appendChild(el("span", "rev-blank"));
  return a;
}
// Line mode: the back lines revealed so far.
function renderLines(card) {
  const wrap = el("div", "rev-lines");
  const lines = card.back || [];
  const shown = Math.min(revealed, lines.length);
  appendBackLines(wrap, card, shown, "div", "rev-line");
  return wrap;
}

function appendBackLines(parent, card, shown, tag, cls) {
  const lines = card.back || [];
  const runs = card.back_runs || [];
  let i = 0;
  while (i < shown) {
    const fence = lines[i].trim().match(/^(```|~~~)/);
    if (fence) {
      const marker = fence[1];
      const code = [];
      i++;
      while (i < shown && lines[i].trim() !== marker) {
        code.push(lines[i]);
        i++;
      }
      if (i < shown) i++;
      const pre = el("pre", "why-code");
      pre.appendChild(el("code", null, code.join("\n")));
      parent.appendChild(pre);
      continue;
    }
    const line = el(tag, cls);
    if (runs[i]) appendRuns(line, runs[i]); else line.textContent = lines[i];
    parent.appendChild(line);
    i++;
  }
}

// Tap-the-answer options. Before a pick each is tappable; after a pick (chosen
// = ChooseFeedbackDto) the correct one greens, a wrong pick reds, the rest dim.
function renderOptions() {
  const wrap = el("div", "rev-options");
  const opts = state.choices || [];
  opts.forEach((label, i) => {
    const b = el("button", "opt-btn");
    b.type = "button";
    if (state.choice_runs && state.choice_runs[i]) appendRuns(b, state.choice_runs[i]);
    else b.textContent = label;
    if (chosen) {
      b.disabled = true;
      if (i === chosen.correct) b.classList.add("opt-correct");
      else if (i === chosen.chosen) b.classList.add("opt-wrong");
      else b.classList.add("opt-dim");
    } else {
      b.addEventListener("click", () => choose(i));
    }
    wrap.appendChild(b);
  });
  return wrap;
}

// The mascot speaks the card's note (the "why"). NoteUnitDto is a tagged union:
// {kind:"sentence",text} → a spoken line; {kind:"code",lines} → a small block.
// An empty/absent note shows nothing (no empty bubble).
function renderWhy(parent, card) {
  const units = card.note || [];
  const keypoints = state.keypoints || [];
  if (!units.length && !keypoints.length) return;
  const row = el("div", "rev-why");
  row.appendChild(mascotEl("mascot-sm"));
  const txt = el("div", "rev-why-text");
  for (const u of units) {
    if (u.kind === "sentence") {
      const paragraph = el("p");
      if (u.runs) appendRuns(paragraph, u.runs); else paragraph.textContent = u.text;
      txt.appendChild(paragraph);
    }
    else if (u.kind === "code") {
      const pre = el("pre", "why-code");
      pre.appendChild(el("code", null, (u.lines || []).join("\n")));
      txt.appendChild(pre);
    }
    else if (u.kind === "checklist") appendChecklist(txt, u.items);
  }
  if (keypoints.length) {
    const list = el("ul", "rev-keypoints");
    keypoints.forEach((point, i) => {
      const item = el("li");
      if (state.keypoint_runs && state.keypoint_runs[i]) appendRuns(item, state.keypoint_runs[i]);
      else item.textContent = point;
      list.appendChild(item);
    });
    txt.appendChild(list);
  }
  row.appendChild(txt);
  parent.appendChild(row);
}

// Home (left) · reveal/rate (centre) · Ask Alix (right) -- Home and Ask Alix
// persist on every card. No score, no "X of N" counter anywhere.
function renderReviewBar(done, acquire, lineMode, choiceMode) {
  const left = el("div", "bar-left");
  const home = el("button", "ghost-home", "🏠 Home");
  home.type = "button";
  home.addEventListener("click", homeFromReview);
  left.appendChild(home);

  const mid = el("div", "bar-mid");
  if (!done) {
    // Still attempting. In choice mode the answer is tapped in the card itself,
    // so the centre stays empty; the other modes get their reveal control.
    if (!choiceMode) {
      const lbl = lineMode ? (revealed === 0 ? "Show me 👀" : "Show me next 👀") : "Show me 👀";
      mid.appendChild(barBtn(lbl, "show-btn", reveal));
    }
  } else if (acquire) {
    // Attempted, but never seen before: the engine grades nothing on a first
    // meeting -- acknowledge it and move on (POST /api/acquire).
    mid.appendChild(barBtn("Got it! Next", "show-btn", acquireNext));
  } else if (choiceMode) {
    // Tap-the-answer: chosen.passed is the engine's truth (ChooseFeedbackDto),
    // never something the UI computes. A correct pick may self-demote via the
    // quiet "I guessed" override, mirroring review.html's isRecognizeMc() -- but
    // a wrong pick has no path to "passed": the correct option is already
    // highlighted on the card, so the single action here honestly records the miss.
    if (chosen.passed) {
      mid.appendChild(barBtn("✅ Got it!", "rate-btn rate-got", () => grade("passed")));
      mid.appendChild(barBtn("🙈 I guessed", "rate-quiet", () => grade("failed")));
    } else {
      mid.appendChild(barBtn("Keep going 🔁", "rate-btn rate-again", () => grade("failed")));
    }
  } else {
    // Revealed a self-assessed card (fill-in-the-blank / line): the kid grades
    // their own retrieval -- "partly" is real at Recall, unlike boolean Recognize.
    mid.appendChild(barBtn("🔁 Try again", "rate-btn rate-again", () => grade("failed")));
    mid.appendChild(barBtn("💪 Almost", "rate-btn rate-almost", () => grade("partly")));
    mid.appendChild(barBtn("✅ Got it!", "rate-btn rate-got", () => grade("passed")));
  }

  const right = el("div", "bar-right");
  const ask = el("button", "ask-btn", "💬 Ask Alix");
  ask.type = "button";
  ask.addEventListener("click", openTutor);
  right.appendChild(ask);

  actionbar.appendChild(left);
  actionbar.appendChild(mid);
  actionbar.appendChild(right);
}

function barBtn(text, cls, fn) {
  const b = el("button", cls, text);
  b.type = "button";
  b.addEventListener("click", fn);
  return b;
}

// ── Review actions (thin -- the engine owns scheduling/grading) ────────────
// Reveal the answer: line mode steps one line; other modes jump to the full
// answer. Just view state -- nothing is recorded until a grade.
function reveal() {
  // Seeing a new card's answer counts as the encounter even if the session
  // ends here (same rule as the adult client). Fire-and-forget.
  if (revealed === 0 && state.acquire) api("/api/reveal", post({})).catch(() => {});
  syncStudyModel(revealKidsAnswer(studyModel));
  render();
}
// A pick is evidence only (ChooseFeedbackDto, discloses the correct index); the
// grade is separate, via the rate bar / /api/grade. Same card stays on screen.
function choose(i) {
  if (state.acquire && !chosen) api("/api/reveal", post({})).catch(() => {});
  api("/api/choose", post({ index: i, card: state.card.id })).then((f) => {
    syncStudyModel(chooseKidsAnswer(studyModel, f));
    render();
  }).catch(resyncState);
}
// The rate bar. Try again → failed, Almost → partly, Got it → passed. /api/grade
// is authoritative: it records and returns the next card (or the done state).
function grade(g) {
  api("/api/grade", post({ grade: g })).then(applyState).catch(resyncState);
}
// Acknowledge a never-seen card (no rating).
function acquireNext() {
  api("/api/acquire", post({})).then(applyState).catch(resyncState);
}
// Leave the session for Home: deselect on the server, then re-scan the boxes.
function homeFromReview() {
  screen = "home";
  currentBox = null;
  currentDeck = null;
  selectError = false;
  syncStudyModel(clearKidsStudyState(studyModel));
  render();
  api("/api/deselect", post({})).catch(() => {}).then(refresh);
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
  api("/api/ask", post({ question: q })).then((d) => { askData = d; if (askOpen) syncAsk(); startAskPoll(); }).catch(resyncState);
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
  if (e.key === "?" && !askOpen && screen === "review" && state && state.kind === "review" && state.card) openTutor();
});

// A trace deck (a WalkDto) can't be walked in kids v1 -- bow out gently instead
// of mis-rendering it. Doubles as the empty/lost-session fallback.
function renderNotReady() {
  const wrap = el("div", "notready");
  wrap.appendChild(mascotEl());
  wrap.appendChild(el("div", "notready-title", "This one isn't ready for kids yet 🌱"));
  wrap.appendChild(el("div", "soft", "Let's pick another box!"));
  stage.appendChild(wrap);
  const back = el("button", "cta-btn", "🏠 Home");
  back.type = "button";
  back.addEventListener("click", homeFromReview);
  actionbar.appendChild(back);
}

// The retrospective Done screen: a bobbing Alix (bob gated by reduced-motion
// via the shared .mascot/kidsBreathe pattern), the honest review count, and
// two exits. No score/streak/XP -- orientation lives only here, once the
// session is over (the no-counter rule is about DURING review).
function renderDone() {
  const wrap = el("div", "done");
  wrap.appendChild(mascotEl("mascot-lg"));
  wrap.appendChild(el("div", "done-title", "Nice work! 🎉"));
  const n = (state && typeof state.reviews === "number") ? state.reviews : 0;
  wrap.appendChild(el("div", "done-count", "You reviewed " + n + (n === 1 ? " card." : " cards.")));
  wrap.appendChild(el("div", "done-sub", "Come back tomorrow to keep them fresh 🌱"));
  stage.appendChild(wrap);

  const actions = el("div", "done-actions");
  if (state && state.can_restart) {
    // "Go again" keeps draining due cards; when only new ones are left, say so
    // rather than silently planting them.
    const goLabel = (state.due_left || 0) > 0 ? "Go again" : "Start new cards";
    const go = el("button", "done-go-btn", goLabel);
    go.type = "button";
    go.addEventListener("click", restartBox);
    actions.appendChild(go);
  }
  const home = el("button", "done-home-btn", "Home");
  home.type = "button";
  home.addEventListener("click", homeFromReview);
  actions.appendChild(home);
  actionbar.appendChild(actions);
}
// Restart the just-finished box: /api/restart returns the next StateDto,
// routed through the same applyState choke point as every other review action.
function restartBox() {
  api("/api/restart", post({})).then(applyState).catch(resyncState);
}

// ── Stage fade hints (hidden scrollbar → soft "▾ more") ───────────────────
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
function loadDecks() {
  return api("/api/decks")
    .then((d) => { deckList = d; loadError = false; })
    .catch(() => { loadError = true; })
    .finally(() => { if (screen === "home") render(); });
}
function refresh() { return loadDecks(); }

Object.assign(window, { el, frontPrompt, contextLine, answerFill, renderOptions, renderWhy, stage });
Object.defineProperty(window, "state", {
  configurable: true,
  get: () => state,
  set: (next) => syncStudyModel({ ...studyModel, state: next }),
});

buildThemeSwatches();
render();     // paints the splash immediately
loadDecks();  // then fills the boxes
