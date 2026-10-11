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
  const card = model.state && model.state.card;
  if (!card) return 0;
  const front = card.front_steps || [];
  return front.length || (card.answer_steps || []).length;
}

export function kidsFilledSteps(model) {
  const steps = (model.state && model.state.card && model.state.card.front_steps) || [];
  if (!steps.length) return 0;
  if (kidsChoiceMode(model)) return model.chosen !== null ? steps.length : 0;
  if (model.state.mode === "line") return Math.min(model.revealed, steps.length);
  return model.revealed > 0 ? steps.length : 0;
}

export function kidsQuestionAt(card, filled) {
  return filled > 0 ? card.front_steps[filled - 1] : card;
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
