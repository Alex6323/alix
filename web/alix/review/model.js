export function createModel(storage) {
  let drawToggle = false;
  try {
    drawToggle = storage.getItem("alix-draw") === "1";
  } catch (_) {}
  return {
    state: null,
    browsing: null,
    walk: null,
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
  return { ...model, state: null, browsing: null, walk: null };
}

export function currentScreen(model) {
  if (model.walk?.kind === "walk") return "walk";
  if (model.browsing) return "browse";
  if (model.state?.kind !== "review") return "picker";
  if (model.state.phase === "done") return "summary";
  if (model.state.phase === "review") return "study";
  if (model.state.phase === "browse") return "browse";
  return "picker";
}
