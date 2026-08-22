import fs from "node:fs";
import path from "node:path";
import { test, expect } from "./helpers";
import { adultDeckRow, openApp } from "./helpers";

const DRAW_WORKSPACE = path.join(
  __dirname,
  "..",
  ".tmp",
  "adult",
  "decks",
  "draw-comparison",
);

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await openApp(page);
});

test.afterEach(async ({ request }) => {
  await request.post("/api/deselect", { data: {} });
  fs.rmSync(DRAW_WORKSPACE, { recursive: true, force: true });
});

test("a revealed drawing sits beside the expected answer", async ({ page }) => {
  fs.mkdirSync(path.join(DRAW_WORKSPACE, "decks"), { recursive: true });
  fs.writeFileSync(path.join(DRAW_WORKSPACE, "alix.toml"), 'title = "Draw Comparison"\n');
  fs.writeFileSync(
    path.join(DRAW_WORKSPACE, "decks", "tcp-stack.md"),
    `---
format-version: 1
id: "deck-00000000000000000000000013"
title: "TCP Stack"
---
## Sketch the layers of a TCP/IP stack <!-- id: card-drawcompare1 --> <!-- input: draw -->
Application, transport, internet, link.
`,
  );

  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Draw Comparison").click();
  await adultDeckRow(page, "TCP Stack").click();
  await page.getByTitle("choose a depth").click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: /^Recall/ }).click(),
  ]);

  const canvas = page.locator(".draw-canvas");
  const canvasBox = await canvas.boundingBox();
  expect(canvasBox).not.toBeNull();
  if (canvasBox) {
    await page.mouse.move(canvasBox.x + 80, canvasBox.y + 80);
    await page.mouse.down();
    await page.mouse.move(canvasBox.x + 220, canvasBox.y + 130, { steps: 4 });
    await page.mouse.up();
  }
  await page.getByRole("button", { name: "Reveal" }).click();

  await expect.soft(page.getByText("Your answer", { exact: true })).toBeVisible();
  await expect.soft(page.getByText("Expected answer", { exact: true })).toBeVisible();

  const attempt = await page.locator(".draw-frozen").boundingBox();
  const expected = await page.locator(".reveal").boundingBox();
  expect(attempt).not.toBeNull();
  expect(expected).not.toBeNull();
  if (attempt && expected) {
    const horizontalGap = expected.x - (attempt.x + attempt.width);
    const verticalOverlap = Math.min(attempt.y + attempt.height, expected.y + expected.height) -
      Math.max(attempt.y, expected.y);
    expect.soft(horizontalGap, "expected answer must be right of the drawing").toBeGreaterThanOrEqual(12);
    expect.soft(verticalOverlap, "drawing and expected answer must share a row").toBeGreaterThan(0);
  }
});
