export function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

export function isReviewState(value) {
  return isRecord(value)
    && value.kind === "review"
    && ["select", "review", "done", "browse"].includes(value.phase);
}

export function isWalkState(value) {
  return isRecord(value) && value.kind === "walk" && typeof value.phase === "string";
}

export function isStudyState(value) {
  return isReviewState(value) || isWalkState(value);
}

export function hasPhase(value) {
  return isRecord(value) && typeof value.phase === "string";
}

export function validatorFor(path) {
  const route = String(path).split("?", 1)[0];
  switch (route) {
    case "/api/state":
    case "/api/select":
    case "/api/deselect":
    case "/api/grade":
    case "/api/skip":
    case "/api/introduce":
    case "/api/remove":
    case "/api/restart":
    case "/api/exam/close":
    case "/api/augment/close":
    case "/api/walk/leave":
      return isStudyState;
    case "/api/walk":
    case "/api/walk/predict":
    case "/api/walk/grade":
    case "/api/walk/restart":
      return isWalkState;
    default:
      return undefined;
  }
}
