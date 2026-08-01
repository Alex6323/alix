export function createKidsStudyModel() {
  return { state: null, revealed: 0, chosen: null };
}

export function applyKidsStudyState(model, state) {
  return { ...model, state, revealed: 0, chosen: null };
}

export function clearKidsStudyState(model) {
  return { ...model, state: null };
}

export function chooseKidsAnswer(model, chosen) {
  return { ...model, chosen };
}

export function revealKidsAnswer(model) {
  const count = kidsBackCount(model);
  const revealed = model.state && model.state.mode === "line"
    ? Math.min(model.revealed + 1, count)
    : count;
  return { ...model, revealed };
}

export function kidsBackCount(model) {
  const back = model.state && model.state.card && model.state.card.back;
  return back ? back.length : 0;
}

export function kidsChoiceMode(model) {
  return !!(model.state && model.state.mode === "choice" && Array.isArray(model.state.choices));
}

export function kidsRevealDone(model) {
  if (kidsChoiceMode(model)) return model.chosen !== null;
  if (model.state.mode === "line") return model.revealed >= kidsBackCount(model);
  return model.revealed > 0;
}

export function kidsStudyScreen(model) {
  const state = model.state;
  return state && (state.kind !== "review" || state.phase !== "done") ? "review" : "done";
}
