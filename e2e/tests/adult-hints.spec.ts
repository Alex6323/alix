// Staged hints in the adult client: a `> [!TIP]` above the divider is offered
// one hint at a time while the answer is still being produced, the first only
// after the server has recorded the opening, and a pass graded after that
// records Partial. The temporary workspace is rewritten per test and removed
// afterwards, progress store included.
import fs from "node:fs";
import path from "node:path";
import type { Page } from "@playwright/test";
import { test, expect } from "./helpers";
import { adultDeckRow, openApp } from "./helpers";

const HINTS_WORKSPACE = path.join(__dirname, "..", ".tmp", "adult", "decks", "hints-lab");

const DECKS: Record<string, string> = {
  "france.md": `---
format-version: 1
id: "deck-00000000000000000000000061"
title: "France"
---
## Capital of France?
> [!TIP]
> A river city.

> [!TIP]
> Starts with P.

---
Paris
<!-- id: card-hintfrance01 -->
`,
  "europe.md": `---
format-version: 1
id: "deck-00000000000000000000000062"
title: "Europe"
---
# Southern capitals

Capitals of the countries around the Mediterranean.

## Capital of Italy?
> [!TIP]
> Founded on seven hills.

> [!TIP]
> Starts with R.

---
Rome
<!-- id: card-hintitaly001 -->
`,
  "plain.md": `---
format-version: 1
id: "deck-00000000000000000000000063"
title: "Plain"
---
## Capital of Spain?
Madrid
<!-- id: card-hintspain001 -->
`,
  "steps.md": `---
format-version: 1
id: "deck-00000000000000000000000064"
title: "Steps"
---
## The first two planets, in order
> [!TIP]
> Count outward from the Sun.

---
Mercury
Venus
<!-- reveal: line -->
<!-- id: card-hintsteps001 -->
`,
};

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  fs.mkdirSync(path.join(HINTS_WORKSPACE, "decks"), { recursive: true });
  fs.writeFileSync(path.join(HINTS_WORKSPACE, "alix.toml"), 'title = "Hints Lab"\n');
  for (const [name, body] of Object.entries(DECKS)) {
    fs.writeFileSync(path.join(HINTS_WORKSPACE, "decks", name), body);
    const reset = await request.post("/api/reset", { data: { deck: `hints-lab/${name}` } });
    expect(reset.ok(), `reset ${name}: ${await reset.text()}`).toBeTruthy();
  }
  await openApp(page);
  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Hints Lab").click();
});

test.afterEach(async ({ request }) => {
  await request.post("/api/deselect", { data: {} });
  fs.rmSync(HINTS_WORKSPACE, { recursive: true, force: true });
});

async function start(page: Page, deck: string, depth: "Recall" | "Reconstruct", cram = false) {
  await adultDeckRow(page, deck).click();
  await page.getByTitle("choose a depth").click();
  if (cram) await page.getByRole("button", { name: /cram/i }).click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: new RegExp(`^${depth}`) }).click(),
  ]);
  await expect(page.locator(".front-text")).toBeVisible();
}

// A first encounter is introduced, never graded; acknowledging it and
// cramming the deck brings the same card back as a real check.
async function introduceThenCram(page: Page, deck: string, depth: "Recall" | "Reconstruct") {
  await start(page, deck, depth);
  await page.getByRole("button", { name: "Reveal" }).click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/introduce")),
    page.getByRole("button", { name: "Seen" }).click(),
  ]);
  await expect(page.getByText("session complete", { exact: true })).toBeVisible();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/deselect")),
    page.getByRole("button", { name: /^Leave/ }).click(),
  ]);
  await start(page, deck, depth, true);
  await expect(page.getByRole("button", { name: "Seen" })).toHaveCount(0);
}

function hintRequests(page: Page): string[] {
  const seen: string[] = [];
  page.on("request", (request) => {
    if (request.url().includes("/api/hint")) seen.push(request.postData() ?? "");
  });
  return seen;
}

const hintChip = (page: Page) => page.locator("#legend .chip", { hasText: /^Hint/ });
const stacked = (page: Page) => page.locator(".section-drawer .hint-stack .note");
const nextHint = (page: Page) => page.getByRole("button", { name: "Next hint" });

test("hints open one at a time, only after the server records the opening", async ({ page }) => {
  const requests = hintRequests(page);
  let release: () => void = () => {};
  const held = new Promise<void>((resolve) => { release = resolve; });
  await page.route("**/api/hint", async (route) => {
    await held;
    await route.continue();
  });

  await start(page, "France", "Recall");
  await expect(hintChip(page), "before the reveal the chip is offered").toBeVisible();
  await expect(hintChip(page).locator(".k")).toHaveText("i");
  const cardId = await page.evaluate(() => fetch("/api/state").then((r) => r.json()).then((s) => s.card.id));

  const [hintRequest] = await Promise.all([
    page.waitForRequest((request) => request.url().includes("/api/hint")),
    page.keyboard.press("i"),
  ]);
  expect(hintRequest.method()).toBe("POST");
  expect(hintRequest.postDataJSON()).toEqual({ card: cardId });
  expect(hintRequest.headers()["x-alix-study-revision"], "the hint echoes the study revision").toMatch(/^\d+$/);
  await expect(page.locator(".section-drawer"), "held request: no panel yet").toHaveCount(0);
  await expect(page.getByText("A river city."), "held request: no hint text yet").toHaveCount(0);

  const [hintResponse] = await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/hint")),
    Promise.resolve().then(release),
  ]);
  expect(hintResponse.status()).toBe(204);
  await expect(stacked(page), "first hint after the 204").toHaveCount(1);
  await expect(stacked(page).nth(0)).toContainText("A river city.");

  await nextHint(page).click();
  await expect(stacked(page), "Next hint stacks the second under the first").toHaveCount(2);
  await expect(stacked(page).nth(0)).toContainText("A river city.");
  await expect(stacked(page).nth(1)).toContainText("Starts with P.");
  await expect(nextHint(page), "no Next hint after the last").toHaveCount(0);

  await page.keyboard.press("Escape");
  await expect(page.locator(".section-drawer"), "Escape closes the panel").toHaveCount(0);
  await page.keyboard.press("i");
  await expect(stacked(page), "reopening shows both opened hints").toHaveCount(2);
  await page.keyboard.press("i");
  await expect(page.locator(".section-drawer"), "the open kind's key closes").toHaveCount(0);
  expect(requests, "one request per card").toHaveLength(1);

  await page.locator("#kebab").click();
  await expect(page.locator("#mHint"), "menu offers Hint before the reveal").toBeVisible();
  await expect(page.locator("#mHintKey")).toHaveText("i");
  await page.locator("#mHint").click();
  await expect(stacked(page), "the menu entry opens the panel").toHaveCount(2);
  await page.keyboard.press("Escape");
  await expect(page.locator(".section-drawer")).toHaveCount(0);

  await page.keyboard.press("Space");
  await expect(page.locator("#ansRegion")).toContainText("Paris");
  await expect(hintChip(page), "after the reveal the chip leaves").toHaveCount(0);
  await page.locator("#kebab").click();
  await expect(page.locator("#mHint"), "after the reveal the menu entry leaves").toBeHidden();
  await page.locator("#kebab").click();
  await page.keyboard.press("i");
  await expect(page.locator(".section-drawer"), "after the reveal `i` opens nothing").toHaveCount(0);
  expect(requests).toHaveLength(1);
});

test("section and hints share one panel and swap without losing opened hints", async ({ page }) => {
  const requests = hintRequests(page);
  await start(page, "Europe", "Recall");
  const drawer = page.locator(".section-drawer");
  const heading = drawer.getByRole("heading", { name: "Southern capitals" });
  await expect(heading, "the first section card opens its section by itself").toBeVisible();

  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/hint")),
    page.keyboard.press("i"),
  ]);
  await expect(stacked(page), "`i` swaps the auto-opened section for the hint").toHaveCount(1);
  await expect(heading).toHaveCount(0);
  await expect(drawer).toHaveCount(1);

  await nextHint(page).click();
  await expect(stacked(page)).toHaveCount(2);

  await page.keyboard.press("c");
  await expect(heading, "`c` swaps to the section").toBeVisible();
  await expect(stacked(page)).toHaveCount(0);
  await page.keyboard.press("i");
  await expect(stacked(page), "`i` swaps back with both hints still opened").toHaveCount(2);
  await expect(stacked(page).nth(1)).toContainText("Starts with R.");
  await page.keyboard.press("c");
  await page.keyboard.press("c");
  await expect(drawer, "the open kind's key closes").toHaveCount(0);
  expect(requests, "swapping sends no further request").toHaveLength(1);

  await page.getByRole("button", { name: /^Context/ }).click();
  await expect(heading).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(drawer).toHaveCount(0);
});

test("typing `i` into an answer field types it and opens nothing", async ({ page }) => {
  await introduceThenCram(page, "France", "Reconstruct");
  const requests = hintRequests(page);
  const field = page.locator("#ansRegion input.field");
  await expect(field).toHaveCount(1);
  await expect(hintChip(page), "a typed check still offers the hint").toBeVisible();

  await field.focus();
  await page.keyboard.type("ipi");
  await expect(field).toHaveValue("ipi");
  await expect(page.locator(".section-drawer")).toHaveCount(0);
  expect(requests).toHaveLength(0);
});

test("a pass graded right after opening a hint records Partial", async ({ page }) => {
  await introduceThenCram(page, "France", "Recall");
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/hint") && response.status() === 204),
    hintChip(page).click(),
  ]);
  await expect(stacked(page)).toHaveCount(1);
  await page.keyboard.press("Escape");
  await expect(page.locator(".section-drawer")).toHaveCount(0);

  await page.keyboard.press("Space");
  const [gradeResponse] = await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/grade") && response.request().method() === "POST"),
    page.keyboard.press("n"),
  ]);
  expect(gradeResponse.status()).toBe(200);
  const graded = await gradeResponse.json();
  expect(graded.partial, `grade response after a hint: ${JSON.stringify(graded)}`).toBe(1);
  expect(graded.reviews).toBe(1);
});

test("a hint-free card shows no chip, and a line reveal keeps it until the last line", async ({ page }) => {
  await start(page, "Plain", "Recall");
  await expect(page.getByRole("button", { name: "Reveal" })).toBeVisible();
  await expect(hintChip(page), "no hints, no chip").toHaveCount(0);
  await page.keyboard.press("i");
  await expect(page.locator(".section-drawer")).toHaveCount(0);

  await page.request.post("/api/deselect", { data: {} });
  await openApp(page);
  await adultDeckRow(page, "Hints Lab").click();
  await start(page, "Steps", "Recall");
  await expect(hintChip(page)).toBeVisible();
  await page.getByRole("button", { name: "Reveal" }).click();
  await expect(page.locator("#ansRegion")).toContainText("Mercury");
  await expect(hintChip(page), "one line out, one to go: still offered").toBeVisible();
  await page.getByRole("button", { name: /^Reveal/ }).click();
  await expect(page.locator("#ansRegion")).toContainText("Venus");
  await expect(hintChip(page), "the last line is out: the chip leaves").toHaveCount(0);
});

test("the hint panel with two hints fits phone and desktop widths", async ({ page }) => {
  await start(page, "France", "Recall");
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/hint")),
    page.keyboard.press("i"),
  ]);
  await nextHint(page).click();
  await expect(stacked(page)).toHaveCount(2);
  const settle = () => page.evaluate(async () => {
    await Promise.all(document.getAnimations().map((animation) => animation.finished.catch(() => undefined)));
  });
  const overflow = () => page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  for (const width of [390, 1200]) {
    await page.setViewportSize({ width, height: 800 });
    await page.keyboard.press("Escape");
    await expect(page.locator(".section-drawer")).toHaveCount(0);
    const closed = await overflow();
    await page.keyboard.press("i");
    await expect(stacked(page)).toHaveCount(2);
    await settle();
    expect(await overflow(), `the open panel adds no horizontal overflow at ${width}px`).toBeLessThanOrEqual(closed);
    if (process.env.HINT_SCREENSHOTS) {
      await page.screenshot({ path: `${process.env.HINT_SCREENSHOTS}/hints-web-${width}.png` });
    }
  }
});
