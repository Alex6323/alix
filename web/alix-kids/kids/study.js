export function createKidsStudy({
  api,
  post,
  model,
  rerender,
  openTutor,
  openPicker,
  refreshPicker,
  reportError,
  ui,
}) {
const {
  actionbar,
  appendChecklist,
  appendRuns,
  contextLine,
  document: doc,
  el,
  frontPrompt,
  mascot: mascotEl,
  stage,
} = ui;
let studyModel = model.create();
let { state, revealed, chosen } = studyModel;

// Mirrors the adult client: `save_error` is stateful, so the banner shows
// exactly as long as the server keeps reporting it. Raw error on the tooltip.
function syncSaveAlert() {
  let a = doc.getElementById("save-alert");
  const msg = state && state.save_error;
  if (!msg) { if (a) a.remove(); return; }
  if (!a) {
    a = doc.createElement("div");
    a.id = "save-alert";
    doc.body.appendChild(a);
  }
  a.title = msg;
  a.textContent = "Uh oh, your progress isn't saving. Ask a grown-up to help!";
}
// ── The review loop ───────────────────────────────────────────────────────
// Every review action (/api/select, /api/grade, /api/introduce, /api/deselect)
// returns the NEXT StateDto -- apply it, reset the per-card view state, and route.
// `/api/select` and each action can also return a WalkDto (a trace deck); kids
// v1 handles only the review StateDto, so we branch on `kind` and route a
// non-review payload to a gentle "not ready" screen (rendered by renderReview).
function apply(s) {
  syncStudyModel(model.apply(studyModel, s));
  rerender();
}

function syncStudyModel(next) {
  studyModel = next;
  ({ state, revealed, chosen } = studyModel);
}
function backCount() { return model.backCount(studyModel); }
function isChoiceMode() { return model.choiceMode(studyModel); }
// Has the answer been fully revealed (so the mascot "why" + rate bar show)?
function revealDone() {
  return model.revealDone(studyModel);
}

// The heart of the app. Branches on kind (trace → not-ready), then on
// state.mode: choice → tap-the-answer, line → reveal-next, everything else →
// fill-in-the-blank. The persistent Home / Ask Alix bar renders for every card.
function renderReview() {
  // Kids handle only the review StateDto; a trace deck resolves to a WalkDto.
  if (!state || state.kind !== "review" || !state.card) { renderNotReady(); return; }

  const card = state.card;
  const introducing = !!state.introducing;
  const choiceMode = isChoiceMode();
  const lineMode = state.mode === "line";
  // A never-seen card is ATTEMPTED like any other -- attempt-first, as the
  // engine intends (it ships `choices` on introduction cards too, and /api/choose
  // answers them). Only the bar differs: one ungraded "Got it! Next" instead of
  // a rate. Forcing `done` here would skip the attempt entirely and make the
  // depth the kid just chose ("Tap the answer" / "Say it yourself") meaningless
  // for the whole first pass through a new deck.
  const done = revealDone();

  const inner = el("div", "rev-stage-inner");
  const cardEl = el("div", "rev-card");

  cardEl.appendChild(el("div", "rev-eyebrow", eyebrowFor(state, introducing)));
  cardEl.appendChild(frontPrompt(card));
  appendContextLines(cardEl, card, done);
  appendImages(cardEl, card.images, done);

  if (choiceMode) {
    cardEl.appendChild(renderOptions());
  } else if (lineMode) {
    cardEl.appendChild(renderLines(card));
  } else {
    // Fill-in-the-blank: a blank before reveal, the green answer after.
    cardEl.appendChild(done ? answerFill(card) : blankEl());
  }
  if (done) appendImages(cardEl, card.images_back, done);

  // Reserve the why-slot whenever the card has a note, so filling it on reveal
  // doesn't resize the card (the shell must not jump).
  if ((card.note && card.note.length > 0) || (state.keypoints && state.keypoints.length > 0)) {
    const slot = el("div", "rev-why-slot");
    if (done) renderWhy(slot, card);
    cardEl.appendChild(slot);
  }

  inner.appendChild(cardEl);
  stage.appendChild(inner);

  renderReviewBar(done, introducing, lineMode, choiceMode);
}

function eyebrowFor(s, introducing) {
  if (introducing) return "Here's a new one! ✨";
  if (isChoiceMode()) return "Tap the answer 👆";
  if (s.mode === "line") return "Line by line 📖";
  return "Fill in the blank ✏️";
}

// Each side's images render as ordered blocks; `im` is a `{ src, alt }` from
// the card's `images` / `images_back` list.
function appendImages(parent, images, done) {
  for (const im of (images || [])) parent.appendChild(cardImg(im, done));
}

// The three-role vocabulary: an asked region shows the blank glyph, a
// sibling card's mask the hidden glyph, and a cover stays a plain fill:
// it hides answer-giving content and is never a question.
function maskGlyph(mask, r, prefix) {
  if (r.role === "cover") return;
  mask.appendChild(el("span", prefix + "-mask-glyph", r.role === "asked" ? "\u2370" : "\u2b1a"));
}

// A done card lifts its reveal-on-answer masks; sibling masks stay.
function keptRegions(regions, done) {
  return regions.filter((r) => !(done && r.reveal_on_answer));
}

// An asked region clipping to nothing is a broken question and fails loud;
// an empty sibling mask or cover hides nothing that exists, so those stay
// silently dropped. No detail: bad deck data is reported on the surface,
// not as a console error (the adult client's notice names the cause).
function askedGone() {
  reportError();
}

// Masks over an uncropped image ride the standard card image: the img keeps
// its shrink-to-fit layout and each mask is positioned in px over the PAINTED
// rect, re-synced whenever the img resizes.
function maskedImage(img, regions, prefix) {
  const wrap = el("div", prefix + "-wrap");
  wrap.appendChild(img);
  for (const r of regions) {
    const mask = el("div", prefix + "-mask");
    maskGlyph(mask, r, prefix);
    mask.dataset.region = JSON.stringify(r);
    wrap.appendChild(mask);
  }
  let warned = false; // sync re-runs on every resize; the error fires once
  const sync = () => {
    const sw = img.naturalWidth, sh = img.naturalHeight;
    const w = img.clientWidth, h = img.clientHeight;
    if (!sw || !sh || !w || !h) return;
    const scale = Math.min(w / sw, h / sh);
    const ox = img.offsetLeft + (w - sw * scale) / 2;
    const oy = img.offsetTop + (h - sh * scale) / 2;
    for (const mask of wrap.querySelectorAll("[data-region]")) {
      const r = JSON.parse(mask.dataset.region);
      const g = r.unit === "%"
        ? { x: (r.x / 100) * sw, y: (r.y / 100) * sh, w: (r.width / 100) * sw, h: (r.height / 100) * sh }
        : { x: r.x, y: r.y, w: r.width, h: r.height };
      // Partial overlap is valid geometry; the painted mask clips at the
      // source edge instead of floating over neighboring content.
      const x0 = Math.max(0, g.x), y0 = Math.max(0, g.y);
      const x1 = Math.min(sw, g.x + g.w), y1 = Math.min(sh, g.y + g.h);
      const visible = x1 > x0 && y1 > y0;
      if (!visible && r.role === "asked" && !warned) {
        warned = true;
        askedGone();
      }
      mask.style.display = visible ? "" : "none";
      mask.style.left = `${ox + x0 * scale}px`;
      mask.style.top = `${oy + y0 * scale}px`;
      mask.style.width = `${(x1 - x0) * scale}px`;
      mask.style.height = `${(y1 - y0) * scale}px`;
    }
  };
  new ResizeObserver(sync).observe(img);
  if (img.complete) sync(); else img.addEventListener("load", sync);
  return wrap;
}

function cardImg(im, done) {
  const img = doc.createElement("img");
  img.className = "rev-img";
  img.src = im.src;
  img.alt = im.alt || "";
  const regions = im.regions || [];
  if (!regions.length && !im.crop) return img;
  if (!im.crop) return maskedImage(img, keptRegions(regions, done), "rev-img");
  // Region and crop geometry live in the source image's own space, never in
  // crop space: the crop is a viewport, the full-image sheet shifts inside
  // it, and masks sit on the sheet. A reveal-on-answer mask lifts once the
  // card is done; sibling masks stay both sides.
  const box = el("div", "rev-img-box");
  const sheet = el("div", "rev-img-sheet");
  sheet.appendChild(img);
  box.appendChild(sheet);
  for (const r of keptRegions(regions, done)) {
    const mask = el("div", "rev-img-mask");
    maskGlyph(mask, r, "rev-img");
    mask.dataset.region = JSON.stringify(r);
    sheet.appendChild(mask);
  }
  box.style.display = "none"; // nothing to show until the source's size is known
  const place = () => {
    const sw = img.naturalWidth, sh = img.naturalHeight;
    if (!sw || !sh) return;
    box.style.display = "";
    const pct = (r) => r.unit === "%"
      ? { x: r.x, y: r.y, w: r.width, h: r.height }
      : { x: (r.x / sw) * 100, y: (r.y / sh) * 100, w: (r.width / sw) * 100, h: (r.height / sh) * 100 };
    const crop = im.crop ? pct(im.crop) : { x: 0, y: 0, w: 100, h: 100 };
    box.style.aspectRatio = `${(crop.w / 100) * sw} / ${(crop.h / 100) * sh}`;
    sheet.style.width = `${(100 / crop.w) * 100}%`;
    sheet.style.height = `${(100 / crop.h) * 100}%`;
    sheet.style.left = `${(-crop.x / crop.w) * 100}%`;
    sheet.style.top = `${(-crop.y / crop.h) * 100}%`;
    for (const mask of sheet.querySelectorAll(".rev-img-mask")) {
      const r = JSON.parse(mask.dataset.region);
      const g = pct(r);
      const x0 = Math.max(0, g.x), y0 = Math.max(0, g.y);
      const x1 = Math.min(100, g.x + g.w), y1 = Math.min(100, g.y + g.h);
      const visible = x1 > x0 && y1 > y0;
      if (!visible && r.role === "asked") askedGone();
      mask.style.display = visible ? "" : "none";
      mask.style.left = `${x0}%`;
      mask.style.top = `${y0}%`;
      mask.style.width = `${x1 - x0}%`;
      mask.style.height = `${y1 - y0}%`;
    }
  };
  if (img.complete) place(); else img.addEventListener("load", place);
  return box;
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

// The ONE fence walk (the same alignment law as the adult client):
// fence-shaped units arrive in document order, the nth closed fence
// consumes the nth such unit, a resolved diagram replaces its fence in
// place once its closing marker is within the walked lines, and
// everything else renders its own interior as code. `onLine(i)` draws a
// non-fence line in the caller's own style.
function walkFences(parent, lines, shown, units, onLine, makeDiagram) {
  const fenceUnits = (units || []).filter(
    (u) => u.kind === "code" || u.kind === "diagram"
  );
  let fenceIndex = 0;
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
      const closed = i < shown;
      if (closed) i++;
      const unit = fenceUnits[fenceIndex];
      fenceIndex++;
      if (closed && unit && unit.kind === "diagram") {
        parent.appendChild(makeDiagram ? makeDiagram(unit) : plainDiagram(unit));
        continue;
      }
      const pre = el("pre", "why-code");
      pre.appendChild(el("code", null, code.join("\n")));
      parent.appendChild(pre);
      continue;
    }
    onLine(i);
    i++;
  }
}

function appendBackLines(parent, card, shown, tag, cls) {
  const lines = card.back || [];
  const runs = card.back_runs || [];
  walkFences(parent, lines, shown, card.back_units, (i) => {
    const line = el(tag, cls);
    if (runs[i]) appendRuns(line, runs[i]); else line.textContent = lines[i];
    parent.appendChild(line);
  });
}

function plainDiagram(unit) {
  const img = el("img", "diagram");
  img.src = unit.src; img.alt = unit.alt || "";
  img.width = unit.width; img.height = unit.height;
  return img;
}

function appendContextLines(parent, card, done) {
  const lines = card.context || [];
  walkFences(parent, lines, lines.length, card.context_units, (i) => {
    parent.appendChild(contextLine(lines[i], card.context_runs && card.context_runs[i]));
  }, (unit) => {
    // The kids surface re-renders per state: the accessible name and the
    // kept masks are recomputed here, the asked mask dropping once done.
    const img = plainDiagram(unit);
    if (done && unit.revealed_alt) img.alt = unit.revealed_alt;
    const kept = keptRegions(unit.regions || [], done);
    if (!kept.length) return img;
    return maskedImage(img, kept, "rev-img");
  });
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
    else if (u.kind === "diagram") {
      const img = el("img", "diagram");
      img.src = u.src; img.alt = u.alt || "";
      img.width = u.width; img.height = u.height;
      txt.appendChild(img);
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
function renderReviewBar(done, introducing, lineMode, choiceMode) {
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
  } else if (introducing) {
    // Attempted, but never seen before: the engine grades nothing on a first
    // meeting -- acknowledge it and move on (POST /api/introduce).
    mid.appendChild(barBtn("Got it! Next", "show-btn", introduceNext));
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
  syncStudyModel(model.reveal(studyModel));
  rerender();
}
// A pick is evidence only (ChooseFeedbackDto, discloses the correct index); the
// grade is separate, via the rate bar / /api/grade. Same card stays on screen.
function choose(i) {
  api("/api/choose", post({ index: i, card: state.card.id })).then((f) => {
    syncStudyModel(model.choose(studyModel, f));
    rerender();
  }).catch(resync);
}
// The rate bar. Try again → failed, Almost → partly, Got it → passed. /api/grade
// is authoritative: it records and returns the next card (or the done state).
function grade(g) {
  api("/api/grade", post({ grade: g })).then(apply).catch(resync);
}
// Acknowledge a never-seen card (no rating).
function introduceNext() {
  api("/api/introduce", post({})).then(apply).catch(resync);
}
// Leave the session for Home: deselect on the server, then re-scan the boxes.
function homeFromReview() {
  clear();
  openPicker();
  api("/api/deselect", post({})).catch(() => {}).then(refreshPicker);
}
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
// via the shared .mascot/kidsBreathe pattern), the honest work tallies, and
// two exits. No score/streak/XP -- orientation lives only here, once the
// session is over (the no-counter rule is about DURING review).
// The invariant: work done means this screen shows it. Recognize answers
// (MC taps) never count as FSRS reviews, so they arrive in their own
// fields; a zero-valued line is never printed, and a session with no work
// at all does not celebrate.
function renderDone() {
  const wrap = el("div", "done");
  wrap.appendChild(mascotEl("mascot-lg"));
  const reviews = (state && state.reviews) || 0;
  const met = (state && state.introduced) || 0;
  // `passed` includes Partial (a partial is a pass); "exactly right" must
  // not re-count the almosts the next line reports (Codex tenth pass, P1).
  const almost = (state && state.partial) || 0;
  const passed = Math.max(((state && state.passed) || 0) - almost, 0);
  const didSomething = reviews + met > 0;
  wrap.appendChild(el("div", "done-title", didSomething ? "Nice work! 🎉" : "All done for now 🌱"));
  const line = (text) => wrap.appendChild(el("div", "done-count", text));
  if (reviews > 0) line("You reviewed " + reviews + (reviews === 1 ? " card." : " cards."));
  if (passed > 0) line("You got " + passed + (passed === 1 ? " card" : " cards") + " right! 👀");
  if (almost > 0) line("So close on " + almost + (almost === 1 ? " card." : " cards."));
  if (met > 0) line("You met " + met + " new " + (met === 1 ? "card." : "cards."));
  if (!didSomething) line("Nothing to review right now.");
  wrap.appendChild(el("div", "done-sub", didSomething ? "Come back tomorrow to keep them fresh 🌱" : "Come back later!"));
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
  api("/api/restart", post({})).then(apply).catch(resync);
}

function clear() {
  syncStudyModel(model.clear(studyModel));
}

function isOpen() {
  return state !== null;
}

function renderStudy() {
  syncSaveAlert();
  if (model.screen(studyModel) === "done") renderDone();
  else renderReview();
}

function resync() {
  return api("/api/state").then(apply).catch(reportError);
}

function revision() {
  return Number.isSafeInteger(state && state.study_revision) ? state.study_revision : null;
}

function currentState() {
  return state;
}

return { apply, clear, isOpen, render: renderStudy, resync, revision, state: currentState };
}
