export function createKidsStudyModel() {
  return { state: null, revealed: 0, chosen: null, selected: [] };
}

export function applyKidsStudyState(model, state) {
  return { ...model, state, revealed: 0, chosen: null, selected: [] };
}

export function clearKidsStudyState(model) {
  return { ...model, state: null };
}

export function chooseKidsAnswer(model, chosen) {
  return { ...model, chosen };
}

export function toggleKidsChoice(model, index) {
  const selected = model.selected.includes(index)
    ? model.selected.filter((picked) => picked !== index)
    : [...model.selected, index].sort((a, b) => a - b);
  return { ...model, selected };
}

export function revealKidsAnswer(model) {
  const count = kidsStepCount(model);
  const revealed = model.state && model.state.mode === "line"
    ? Math.min(model.revealed + 1, count)
    : count;
  return { ...model, revealed };
}

export function kidsStepCount(model) {
  const steps = model.state && model.state.card && model.state.card.answer_steps;
  return steps ? steps.length : 0;
}

export function kidsChoiceMode(model) {
  return !!(model.state && model.state.mode === "choice" && Array.isArray(model.state.choices));
}

export function kidsMultiMode(model) {
  return kidsChoiceMode(model) && model.state.choices_multiple === true;
}

export function kidsRevealDone(model) {
  if (kidsChoiceMode(model)) return model.chosen !== null;
  if (model.state.mode === "line") return model.revealed >= kidsStepCount(model);
  return model.revealed > 0;
}

export function kidsStudyScreen(model) {
  const state = model.state;
  return state && (state.kind !== "review" || state.phase !== "done") ? "review" : "done";
}
