import assert from "node:assert/strict";
import test from "node:test";

import { createKidsTheme } from "../../web/alix-kids/kids/theme.js";

test("kids theme rejects unknown names and persists known names", () => {
  const stored = new Map([["alix-kids-theme", "__proto__"]]);
  const writes = [];
  const properties = new Map();
  const theme = createKidsTheme({
    storage: {
      getItem: (key) => stored.get(key) ?? null,
      setItem(key, value) {
        writes.push([key, value]);
        stored.set(key, value);
      },
    },
    rootStyle: {
      setProperty: (name, value) => properties.set(name, value),
    },
  });

  assert.equal(theme.current(), "Sunrise");
  assert.equal(theme.set("Midnight"), false);
  assert.equal(theme.current(), "Sunrise");
  assert.equal(theme.palette("__proto__"), null);
  assert.deepEqual(writes, []);

  assert.equal(theme.set("Ocean"), true);
  assert.equal(theme.current(), "Ocean");
  assert.deepEqual(writes, [["alix-kids-theme", "Ocean"]]);
  assert.equal(properties.get("--bg-top"), "#eafaf7");
  assert.equal(properties.get("--bg-bot"), "#cdeeff");
  assert.equal(properties.get("--bg"), "linear-gradient(168deg, #eafaf7 0%, #cdeeff 100%)");
  assert.equal(properties.get("--accent"), "#0fa8b4");
  assert.equal(properties.get("--accent-sh"), "#0b7d86");
});
