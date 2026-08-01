export class KidsApiError extends Error {
  constructor(method, path, status) {
    super(`${method} ${path} → ${status}`);
    this.name = "KidsApiError";
    this.status = status;
  }
}

export function captureKidsPairingToken({ location, history, sessionStorage }) {
  const token = new URLSearchParams(location.search).get("token");
  if (!token) return;
  try { sessionStorage.setItem("alix.token", token); } catch (_) {}
  const url = new URL(location.href);
  url.searchParams.delete("token");
  history.replaceState(null, "", url);
}

export function createKidsApiClient({ fetchImpl, sessionStorage, onUnauthorized, revision }) {
  let unauthorizedShown = false;

  function authenticated(path, options = {}) {
    let token = null;
    try { token = sessionStorage.getItem("alix.token"); } catch (_) {}
    if (!token || !String(path).startsWith("/api")) return options;
    return {
      ...options,
      headers: { ...(options.headers || {}), Authorization: `Bearer ${token}` },
    };
  }

  async function request(path, options) {
    const response = await fetchImpl(path, authenticated(path, options));
    if (response.status === 401 && String(path).startsWith("/api")) {
      let rejected = null;
      try { rejected = sessionStorage.getItem("alix.token"); } catch (_) {}
      if (rejected) {
        try { sessionStorage.removeItem("alix.token"); } catch (_) {}
      }
      if (!unauthorizedShown) {
        unauthorizedShown = true;
        onUnauthorized();
      }
    }
    if (!response.ok) {
      throw new KidsApiError((options && options.method) || "GET", path, response.status);
    }
    return response.json();
  }

  function postOptions(body) {
    const headers = { "Content-Type": "application/json" };
    const currentRevision = revision();
    if (Number.isSafeInteger(currentRevision)) {
      headers["X-Alix-Study-Revision"] = String(currentRevision);
    }
    return { method: "POST", headers, body: JSON.stringify(body) };
  }

  return { request, postOptions };
}

export function createKidsErrorReporter({ console, timers, ui }) {
  let timer = null;

  function dismiss() {
    if (timer !== null) timers.clearTimeout(timer);
    timer = null;
    ui.oops.hidden = true;
  }

  function show(detail) {
    if (detail) console.error("alix kids:", detail);
    ui.oops.textContent = "Oops, that didn't work. Try again? 🌱";
    ui.oops.hidden = false;
    if (timer !== null) timers.clearTimeout(timer);
    timer = timers.setTimeout(dismiss, 6000);
  }

  function handleUnhandledRejection(event) {
    if (event.reason && event.reason.status === 401) return;
    show(event.reason);
  }

  function handleError(event) {
    show(event.error || event.message);
  }

  return { dismiss, handleError, handleUnhandledRejection, show };
}
