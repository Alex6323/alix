import assert from "node:assert/strict";
import test from "node:test";

import { KidsApiError, createKidsApiClient } from "../../web/alix-kids/kids/api.js";

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

test("kids api rejects non success before decoding json", async () => {
  let decodes = 0;
  const client = createKidsApiClient({
    fetchImpl: async () => response(503, {}, () => decodes++),
    sessionStorage: storage(),
    onUnauthorized: () => {},
    revision: () => null,
  });

  await assert.rejects(client.request("/api/state"), (error) => {
    assert.ok(error instanceof KidsApiError);
    assert.equal(error.status, 503);
    assert.equal(error.message, "GET /api/state → 503");
    return true;
  });
  assert.equal(decodes, 0);
});

test("kids api clears a rejected pairing token once", async () => {
  let unauthorized = 0;
  const saved = storage({ "alix.token": "wrong" });
  const client = createKidsApiClient({
    fetchImpl: async () => response(401, {}),
    sessionStorage: saved,
    onUnauthorized: () => unauthorized++,
    revision: () => null,
  });

  await assert.rejects(client.request("/api/state"), KidsApiError);
  await assert.rejects(client.request("/api/state"), KidsApiError);
  assert.equal(saved.getItem("alix.token"), null);
  assert.equal(unauthorized, 1);
});

test("kids post options carry the current study revision", () => {
  const client = createKidsApiClient({
    fetchImpl: async () => response(200, {}),
    sessionStorage: storage(),
    onUnauthorized: () => {},
    revision: () => 41,
  });

  assert.deepEqual(client.postOptions({ grade: "passed" }), {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-Alix-Study-Revision": "41",
    },
    body: '{"grade":"passed"}',
  });
});
