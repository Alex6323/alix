export class ApiError extends Error {
  constructor(path, status, body) {
    super(`request to ${path} failed with status ${status}`);
    this.name = "ApiError";
    this.path = path;
    this.status = status;
    this.body = body;
  }
}

export function capturePairingToken({ location, history, sessionStorage }) {
  const token = new URLSearchParams(location.search).get("token");
  if (!token) return;
  sessionStorage.setItem("alix.token", token);
  const url = new URL(location.href);
  url.searchParams.delete("token");
  history.replaceState(null, "", url);
}

export function createApiClient({ fetchImpl, sessionStorage, onUnauthorized, revision }) {
  let unauthorizedShown = false;

  function authenticated(path, options = {}) {
    const token = sessionStorage.getItem("alix.token");
    if (!token || !String(path).startsWith("/api")) return options;
    return {
      ...options,
      headers: { ...(options.headers || {}), Authorization: `Bearer ${token}` },
    };
  }

  async function fetchResponse(path, options = {}) {
    const response = await fetchImpl(path, authenticated(path, options));
    if (response.status === 401 && String(path).startsWith("/api")) {
      const rejected = sessionStorage.getItem("alix.token");
      if (rejected) sessionStorage.removeItem("alix.token");
      if (!unauthorizedShown) {
        unauthorizedShown = true;
        onUnauthorized();
      }
    }
    if (!response.ok) {
      let body;
      try { body = await response.json(); } catch { body = undefined; }
      throw new ApiError(path, response.status, body);
    }
    return response;
  }

  async function request(path, options, validate) {
    const response = await fetchResponse(path, options);
    let value;
    try {
      value = await response.json();
    } catch (error) {
      throw new Error(`invalid JSON from ${path}`, { cause: error });
    }
    if (validate && !validate(value)) throw new Error(`invalid response from ${path}`);
    return value;
  }

  function get(path, validate) {
    return request(path, undefined, validate);
  }

  function post(path, body, validate) {
    return request(path, postOptions(body), validate);
  }

  function postOptions(body) {
    const headers = { "Content-Type": "application/json" };
    const currentRevision = revision();
    if (Number.isSafeInteger(currentRevision)) {
      headers["X-Alix-Study-Revision"] = String(currentRevision);
    }
    return { method: "POST", headers, body: JSON.stringify(body) };
  }

  function withToken(path) {
    const token = sessionStorage.getItem("alix.token");
    if (!token) return path;
    const separator = path.includes("?") ? "&" : "?";
    return `${path}${separator}token=${encodeURIComponent(token)}`;
  }

  return { fetch: fetchResponse, get, post, postOptions, request, withToken };
}
