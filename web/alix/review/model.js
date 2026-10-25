export function createModel(storage) {
  let drawToggle = false;
  try {
    drawToggle = storage.getItem("alix-draw") === "1";
  } catch (_) {}
  return {
    state: null,
    browsing: null,
    trace: null,
    revealed: 0,
    citationView: false,
    sectionView: false,
    answerConcealed: false,
    feedback: null,
    typelineChecked: [],
    confirmingLeave: false,
    explainInput: "",
    marks: [],
    keypointCursor: 0,
    drawStrokes: [],
    drawSnapshot: null,
    drawTool: "pen",
    drawCanvas: null,
    drawToggle,
    drawerSelection: { deck: null, topology: null, region: null },
    keys: {},
  };
}

export function applyStudyState(model, state) {
  return {
    ...model,
    state,
    revealed: 0,
    citationView: false,
    sectionView: false,
    answerConcealed: false,
    feedback: null,
    typelineChecked: [],
    confirmingLeave: false,
    explainInput: "",
    marks: [],
    keypointCursor: 0,
    drawStrokes: [],
    drawSnapshot: null,
    drawTool: "pen",
    drawCanvas: null,
  };
}

export function enterPicker(model) {
  return { ...model, state: null, browsing: null, trace: null };
}

export function currentScreen(model) {
  if (model.trace?.kind === "trace") return "trace";
  if (model.browsing) return "browse";
  if (model.state?.kind === "walk") return model.state.phase === "done" ? "walk-done" : "walk";
  if (model.state?.kind !== "review") return "picker";
  if (model.state.phase === "done") return "summary";
  if (model.state.phase === "review") return "study";
  if (model.state.phase === "browse") return "browse";
  return "picker";
}

export function revealStepCount(card) {
  if (!card) return 0;
  const front = card.front_steps || [];
  return front.length || (card.answer_steps || []).length;
}

// The hidden look of a revealed answer: each word keeps its first character,
// and its last past two characters; the rest is drawn transparent in place.
// Runs concatenate back to the text, so the line keeps its exact width.
export function veilRuns(text) {
  const runs = [];
  const push = (chunk, kept) => {
    if (!chunk) return;
    const last = runs[runs.length - 1];
    if (last && last.kept === kept) last.text += chunk;
    else runs.push({ text: chunk, kept });
  };
  for (const part of String(text || "").split(/(\s+)/)) {
    if (!part) continue;
    if (/^\s+$/.test(part)) { push(part, true); continue; }
    const chars = Array.from(part);
    if (chars.length <= 2) {
      push(chars[0], true);
      push(chars.slice(1).join(""), false);
      continue;
    }
    push(chars[0], true);
    push(chars.slice(1, -1).join(""), false);
    push(chars[chars.length - 1], true);
  }
  return runs;
}

export function filledSteps({ card, mode, revealed, answered, concealed }) {
  const steps = (card && card.front_steps) || [];
  if (!steps.length || concealed) return 0;
  if (answered) return steps.length;
  if (mode === "line") return Math.min(revealed, steps.length);
  return revealed > 0 ? steps.length : 0;
}

export function questionAt(card, filled) {
  return filled > 0 ? card.front_steps[filled - 1] : card;
}
