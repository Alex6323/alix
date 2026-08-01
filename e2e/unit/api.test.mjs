import assert from "node:assert/strict";
import test from "node:test";

import { ApiError, createApiClient } from "../../assets/web/review/api.js";

function storage(initial = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem: (key) => values.get(key) ?? null,
    removeItem: (key) => values.delete(key),
    setItem: (key, value) => values.set(key, value),
  };
}

function response(status, body, onJson = () => {}) {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => {
      onJson();
      return body;
    },
  };
}

test("api rejects non success before decoding json", async () => {
  let decodes = 0;
  const client = createApiClient({
    fetchImpl: async () => response(503, { phase: "error" }, () => decodes++),
    sessionStorage: storage(),
    onUnauthorized: () => {},
    revision: () => null,
  });

  await assert.rejects(client.get("/api/state"), (error) => {
    assert.ok(error instanceof ApiError);
    assert.equal(error.status, 503);
    return true;
  });
  assert.equal(decodes, 0);
});

test("api rejects a response that fails its route validator", async () => {
  const client = createApiClient({
    fetchImpl: async () => response(200, { kind: "surprise" }),
    sessionStorage: storage(),
    onUnauthorized: () => {},
    revision: () => null,
  });

  await assert.rejects(
    client.get("/api/state", (value) => value.kind === "review"),
    /invalid response from \/api\/state/,
  );
});

test("api attaches the current study revision to mutations", async () => {
  let request;
  const client = createApiClient({
    fetchImpl: async (path, options) => {
      request = { path, options };
      return response(200, { kind: "review" });
    },
    sessionStorage: storage(),
    onUnauthorized: () => {},
    revision: () => 41,
  });

  await client.post("/api/grade", { grade: "passed" });
  assert.equal(request.path, "/api/grade");
  assert.equal(request.options.headers["X-Alix-Study-Revision"], "41");
  assert.equal(request.options.headers["Content-Type"], "application/json");
  assert.equal(request.options.body, '{"grade":"passed"}');
});

test("api clears a rejected pairing token once", async () => {
  let unauthorized = 0;
  let calls = 0;
  const saved = storage({ "alix.token": "wrong" });
  const client = createApiClient({
    fetchImpl: async () => {
      calls++;
      return response(401, {});
    },
    sessionStorage: saved,
    onUnauthorized: () => unauthorized++,
    revision: () => null,
  });

  await assert.rejects(client.get("/api/state"), ApiError);
  await assert.rejects(client.get("/api/state"), ApiError);
  assert.equal(calls, 2);
  assert.equal(saved.getItem("alix.token"), null);
  assert.equal(unauthorized, 1);
});
