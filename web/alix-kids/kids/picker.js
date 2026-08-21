export function kidsCatalogFailed(state) {
  return { ...state, loadError: true };
}

export function createKidsPicker({ api, post, openStudy, rerender, isVisible, ui }) {
const { actionbar, document: doc, el, mascot: mascotEl, stage } = ui;
let currentBox = null;
let currentDeck = null;
let selectError = false;
let deckList = null;
let loadError = false;

function renderHome() {
  // Before the first /api/decks resolves, show the brand-mark splash.
  if (deckList == null && !loadError) {
    const splash = el("div", "splash");
    const logo = doc.createElement("alix-logo");
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
  // A damaged member is deliberately non-reviewable, so plain
  // !reviewable would tell the kid a broken box is complete.
  const damaged = !b.reviewable
    && (b.members || []).some((m) => m.state === "error");
  const ready = b.reviewable ? "ready to practise"
    : damaged ? "needs a grown-up 🔧"
    : "all caught up 🌱";
  card.appendChild(el("div", "box-ready", ready));
  card.addEventListener("click", () => openBox(b));
  return card;
}

// A workspace's emblem when it has one, else a friendly default emoji. `/img`
// URLs are unauthenticated by design, so a plain <img src> is enough.
function iconEl(item, imgCls, emojiCls) {
  if (item && item.icon) {
    const img = doc.createElement("img");
    img.className = imgCls;
    img.src = item.icon;
    img.alt = "";
    return img;
  }
  return el("div", emojiCls, "📚");
}

function openBox(b) { currentBox = b; currentDeck = null; selectError = false; rerender(); }
function goHome() { home(); load(); }
function openDeck(m) { currentDeck = m; selectError = false; rerender(); }
function backToBox() { currentDeck = null; selectError = false; rerender(); }

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
  if (m.state === "error") {
    // An unreadable progress document: honest and calm, no alarm, and no
    // buttons that the server would refuse anyway.
    wrap.appendChild(el("div", "soft", "This one needs a grown-up's help before practising. 🔧"));
  } else {
    wrap.appendChild(el("div", "soft", "How do you want to practise?"));

    const choices = el("div", "depth-choices launch-choices");
    choices.appendChild(depthBtn("👆 Tap the answer", "recognize", m.reviewable_recognize, m.name));
    choices.appendChild(depthBtn("🗣️ Say it yourself", "recall", m.reviewable_recall, m.name));
    wrap.appendChild(choices);
  }

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
  if (m.state === "error") return el("span", "pill-new", "needs a grown-up 🔧");
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
    .then(openStudy)
    .catch(() => { selectError = true; rerender(); });
}

function home() {
  currentBox = null;
  currentDeck = null;
  selectError = false;
  rerender();
}

function isHome() {
  return currentBox === null;
}

function load() {
  return api("/api/decks")
    .then((next) => { deckList = next; loadError = false; })
    .catch(() => {
      ({ deckList, loadError } = kidsCatalogFailed({ deckList, loadError }));
    })
    .finally(() => { if (isVisible()) rerender(); });
}

function renderPicker() {
  if (currentBox) renderBox();
  else renderHome();
}

return { home, isHome, load, render: renderPicker };
}
