// The picker's root-error state, driven through the real server: while the
// scratch decks dir is renamed away, GET /api/decks answers 500 (the root
// read_dir error is no longer swallowed into an empty catalog), and the
// picker shows a calm retryable notice instead of "No decks found". Renaming
// the dir back and clicking Retry recovers without a reload.
import fs from "node:fs";
import path from "node:path";
import { test, expect } from "./helpers";
import { adultDeckRow, openApp } from "./helpers";

const DECKS_DIR = path.join(__dirname, "..", ".tmp", "adult", "decks");
const HIDDEN_DIR = `${DECKS_DIR}-hidden`;

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await openApp(page);
});

// Restore the fixture dir even when an assertion mid-test fails: later specs
// share this server and its decks dir.
test.afterEach(() => {
  if (fs.existsSync(HIDDEN_DIR)) fs.renameSync(HIDDEN_DIR, DECKS_DIR);
});

test("an unreadable decks root shows a calm retryable notice, and retry recovers", async ({ page, pageErrors }) => {
  await expect(adultDeckRow(page, "Animals")).toBeVisible();

  fs.renameSync(DECKS_DIR, HIDDEN_DIR);
  const [errorResponse] = await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/decks")),
    page.locator("#navRefresh").click(),
  ]);
  expect(errorResponse.status()).toBe(500);

  await expect(page.locator(".msg")).toHaveText("Couldn't read the decks folder.");
  const retry = page.getByRole("button", { name: "Retry" });
  await expect(retry).toBeVisible();
  // The wrong state would be the empty-catalog lie; pin its absence.
  await expect(page.getByText("No decks found", { exact: false })).toHaveCount(0);

  fs.renameSync(HIDDEN_DIR, DECKS_DIR);
  const [okResponse] = await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/decks")),
    retry.click(),
  ]);
  expect(okResponse.status()).toBe(200);
  await expect(adultDeckRow(page, "Animals")).toBeVisible();

  // The 500 is this test's expected outcome; scrub exactly its console
  // entries so the pageErrors teardown still catches everything else.
  const expected = pageErrors.filter((e) => e.includes("Failed to load resource"));
  expect(expected.length).toBeGreaterThan(0);
  for (const entry of expected) pageErrors.splice(pageErrors.indexOf(entry), 1);
});
