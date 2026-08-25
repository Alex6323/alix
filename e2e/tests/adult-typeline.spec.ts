// A real-server regression for typing an ordered answer one line at a time.
// The server returned a result for every answer line however few the learner
// had submitted, and the client reads that count to decide whether another
// field is owed, so the first line closed the card and marked the rest wrong.
// The temporary workspace keeps the exact path independent of the shared
// fixtures, then removes it even when an assertion fails.
import fs from "node:fs";
import path from "node:path";
import { test, expect } from "./helpers";
import { adultDeckRow, openApp } from "./helpers";

const TYPELINE_WORKSPACE = path.join(
  __dirname,
  "..",
  ".tmp",
  "adult",
  "decks",
  "typeline-progression",
);

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await openApp(page);
});

test.afterEach(async ({ request }) => {
  await request.post("/api/deselect", { data: {} });
  fs.rmSync(TYPELINE_WORKSPACE, { recursive: true, force: true });
});

test("typing an ordered answer asks for every line, not just the first", async ({ page }) => {
  fs.mkdirSync(path.join(TYPELINE_WORKSPACE, "decks"), { recursive: true });
  fs.writeFileSync(
    path.join(TYPELINE_WORKSPACE, "alix.toml"),
    'title = "Typeline Progression"\n',
  );
  fs.writeFileSync(
    path.join(TYPELINE_WORKSPACE, "decks", "handshake.md"),
    `---
format-version: 1
id: "deck-00000000000000000000000013"
title: "Handshake"
---
## The TCP handshake, in order
SYN
SYN-ACK
<!-- reveal: line -->
<!-- id: card-typeline1 -->
`,
  );

  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Typeline Progression").click();
  await adultDeckRow(page, "Handshake").click();

  // A first encounter is introduced, never tested. Acknowledge it so the next
  // presentation is the real Reconstruct check.
  await page.getByTitle("choose a depth").click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: /^Reconstruct/ }).click(),
  ]);
  await page.getByRole("button", { name: "Reveal" }).click();
  await page.getByRole("button", { name: "Seen" }).click();
  await page.getByRole("button", { name: /^Leave/ }).click();

  await adultDeckRow(page, "Handshake").click();
  await page.getByTitle("choose a depth").click();
  await page.getByRole("button", { name: /cram/i }).click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: /^Reconstruct/ }).click(),
  ]);

  const field = page.locator("#ansRegion input.field");
  await expect(field).toHaveCount(1);

  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/check")),
    field.fill("SYN").then(() => field.press("Enter")),
  ]);

  // The first line is progress, not a verdict: it is checked off, the second
  // line still owes its field, and the card cannot be graded yet.
  await expect(page.locator("#ansRegion .answer.pass")).toHaveText(["SYN✓"]);
  await expect(page.locator("#ansRegion .answer.miss")).toHaveCount(0);
  await expect(page.locator("#ansRegion input.field")).toHaveCount(1);
  await expect(page.getByRole("button", { name: "Got it" })).toHaveCount(0);

  const second = page.locator("#ansRegion input.field");
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/check")),
    second.fill("SYN-ACK").then(() => second.press("Enter")),
  ]);

  await expect(page.locator("#ansRegion .answer.pass")).toHaveText(["SYN✓", "SYN-ACK✓"]);
  await expect(page.getByRole("button", { name: "Got it" })).toBeVisible();
});
