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
