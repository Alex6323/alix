// Reveal holds the footer's centre whether the card offers Context, Hint,
// both, or neither: an absent side chip leaves its slot blank. The temporary
// workspace keeps these cards independent of the shared fixtures.
import fs from "node:fs";
import path from "node:path";
import type { Page } from "@playwright/test";
import { test, expect } from "./helpers";
import { adultDeckRow, openApp } from "./helpers";

const WORKSPACE = path.join(__dirname, "..", ".tmp", "adult", "decks", "legend-slots");

function deck(n: number, topic: boolean, hint: boolean): string {
  return `---\nformat-version: 1\nid: "deck-0000000000000000000000006${n}"\ntitle: "Slots ${n}"\n---\n`
    + (topic ? "# Ownership\nWho frees a value, and when.\n\n" : "")
    + "## What does Rc count?\n"
    + (hint ? "> [!TIP]\n> Think about sharing.\n\n---\n" : "")
    + `The number of owners.\n<!-- id: card-slots${n} -->\n`;
}

async function closeTopic(page: Page): Promise<void> {
  const close = page.getByRole("button", { name: "Close topic context" });
  if (await close.isVisible()) await close.click();
}

async function openAtRecall(page: Page, n: number, topic: boolean, hint: boolean): Promise<void> {
  fs.mkdirSync(path.join(WORKSPACE, "decks"), { recursive: true });
  fs.writeFileSync(path.join(WORKSPACE, "alix.toml"), 'title = "Legend Slots"\n');
  fs.writeFileSync(path.join(WORKSPACE, "decks", `slots${n}.md`), deck(n, topic, hint));
  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Legend Slots").click();
  await adultDeckRow(page, `Slots ${n}`).click();
  await page.getByTitle("choose a depth").click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: /^Recall/ }).click(),
  ]);
  await closeTopic(page);
  await page.getByRole("button", { name: /^Reveal/ }).click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/introduce")),
    page.getByRole("button", { name: /^Seen/ }).click(),
  ]);
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/deselect")),
    page.getByRole("button", { name: /^Leave/ }).click(),
  ]);
  await adultDeckRow(page, `Slots ${n}`).click();
  await page.getByTitle("choose a depth").click();
  await page.getByRole("button", { name: /cram/i }).click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: /^Recall/ }).click(),
  ]);
  await closeTopic(page);
}

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await openApp(page);
});

test.afterEach(async ({ request }) => {
  await request.post("/api/deselect", { data: {} });
  fs.rmSync(WORKSPACE, { recursive: true, force: true });
});

for (const [n, topic, hint] of [[1, true, true], [2, true, false], [3, false, true], [4, false, false]] as const) {
  test(`Reveal holds the centre with topic ${topic} and hint ${hint}`, async ({ page }) => {
    await openAtRecall(page, n, topic, hint);
    const reveal = page.getByRole("button", { name: /^Reveal/ });
    await expect(reveal, "the card waits on Reveal").toBeVisible();
    await expect(page.getByRole("button", { name: /^Context/ }), "Context is offered only with a topic").toHaveCount(topic ? 1 : 0);
    await expect(page.getByRole("button", { name: /^Hint/ }), "Hint is offered only with a hint").toHaveCount(hint ? 1 : 0);
    const button = (await reveal.boundingBox())!;
    const footer = (await page.locator(".legend").boundingBox())!;
    const at = button.x + button.width / 2;
    const centre = footer.x + footer.width / 2;
    expect(Math.abs(at - centre), `Reveal centre ${at} sits on the footer centre ${centre}`).toBeLessThanOrEqual(1);
  });
}

test("a 360px footer keeps Context, Reveal, and Hint separate and on screen", async ({ page }) => {
  await openAtRecall(page, 5, true, true);
  await page.setViewportSize({ width: 360, height: 800 });
  const footer = (await page.locator(".legend").boundingBox())!;
  const context = (await page.getByRole("button", { name: /^Context/ }).boundingBox())!;
  const reveal = (await page.getByRole("button", { name: /^Reveal/ }).boundingBox())!;
  const hint = (await page.getByRole("button", { name: /^Hint/ }).boundingBox())!;
  const disjoint = (a: typeof context, b: typeof context) =>
    a.x + a.width <= b.x || b.x + b.width <= a.x
      || a.y + a.height <= b.y || b.y + b.height <= a.y;
  expect(
    context.x >= footer.x
      && disjoint(context, reveal)
      && disjoint(reveal, hint)
      && disjoint(context, hint)
      && hint.x + hint.width <= footer.x + footer.width,
    `footer ${JSON.stringify(footer)}, Context ${JSON.stringify(context)}, Reveal ${JSON.stringify(reveal)}, Hint ${JSON.stringify(hint)} must be separate and on screen`,
  ).toBe(true);
});
