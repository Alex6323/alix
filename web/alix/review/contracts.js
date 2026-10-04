export function isRecord(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

export function isReviewState(value) {
  return isRecord(value)
    && value.kind === "review"
    && ["select", "review", "done", "browse"].includes(value.phase);
}

export function isTraceSessionState(value) {
  return isRecord(value) && value.kind === "trace" && typeof value.phase === "string";
}

export function isStudyState(value) {
  return isReviewState(value) || isTraceSessionState(value);
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
    case "/api/trace/leave":
      return isStudyState;
    case "/api/trace":
    case "/api/trace/predict":
    case "/api/trace/grade":
    case "/api/trace/restart":
      return isTraceSessionState;
    default:
      return undefined;
  }
}
