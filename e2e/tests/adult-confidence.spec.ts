// Confidence tracking in the adult client: the depth menu's toggle is sent
// with the launch, a graded card asks Sure/Unsure in place of Reveal (or
// after a single-choice pick, in place of an immediate submit), and the mark
// lands on the review in the deck's progress. The temporary workspace is
// rewritten per test and removed afterwards, progress store included.
import fs from "node:fs";
import path from "node:path";
import type { Page } from "@playwright/test";
import { test, expect } from "./helpers";
import { adultDeckRow, openApp } from "./helpers";

const WORKSPACE = path.join(__dirname, "..", ".tmp", "adult", "decks", "confidence-lab");

const DECKS: Record<string, string> = {
  "flip.md": `---
format-version: 1
id: "deck-00000000000000000000000071"
title: "Flip"
---
## Capital of Norway?
Oslo
<!-- id: card-confflip0001 -->
`,
  "pair.md": `---
format-version: 1
id: "deck-00000000000000000000000073"
title: "Pair"
---
## The two Scandinavian capitals west of Finland
Oslo
Stockholm
<!-- id: card-confpair0001 -->
`,
  "blanks.md": `---
format-version: 1
id: "deck-00000000000000000000000074"
title: "Blanks"
---
## Fill in both capitals
Norway's capital is Oslo and Sweden's is Stockholm.
<!-- blank: span [both] hidden="Oslo" -->
<!-- blank: span [both] hidden="Stockholm" -->
<!-- id: card-confblanks001 -->
`,
  "pick.md": `---
format-version: 1
id: "deck-00000000000000000000000072"
title: "Pick"
---
## Capital of Sweden?
- [x] Stockholm
- [ ] Gothenburg
- [ ] Malmo
<!-- choices: single -->
<!-- id: card-confpick0001 -->
`,
};

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  fs.mkdirSync(path.join(WORKSPACE, "decks"), { recursive: true });
  fs.writeFileSync(path.join(WORKSPACE, "alix.toml"), 'title = "Confidence Lab"\n');
  for (const [name, body] of Object.entries(DECKS)) {
    fs.writeFileSync(path.join(WORKSPACE, "decks", name), body);
    const reset = await request.post("/api/reset", { data: { deck: `confidence-lab/${name}` } });
    expect(reset.ok(), `reset ${name}: ${await reset.text()}`).toBeTruthy();
  }
  await openApp(page);
  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Confidence Lab").click();
});

test.afterEach(async ({ request }) => {
  await request.post("/api/deselect", { data: {} });
  fs.rmSync(WORKSPACE, { recursive: true, force: true });
});

type Depth = "Recognize" | "Recall" | "Reconstruct";

async function start(page: Page, deck: string, depth: Depth, opts: { cram?: boolean; track?: boolean; skipIntro?: boolean } = {}) {
  await adultDeckRow(page, deck).click();
  await page.getByTitle("choose a depth").click();
  if (opts.cram) await page.getByRole("button", { name: /cram/i }).click();
  if (opts.skipIntro) {
    await page.keyboard.press("s");
    await expect(page.getByRole("button", { name: /skip intro/i }), "s ticks the box").toContainText("☑");
  }
  if (opts.track) {
    await page.keyboard.press("f");
    await expect(page.getByRole("button", { name: /confidence/i }), "f ticks the box").toContainText("☑");
  }
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: new RegExp(`^${depth}`) }).click(),
  ]);
  await expect(page.locator(".front-text")).toBeVisible();
}

async function leave(page: Page) {
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/deselect")),
    page.getByRole("button", { name: /^Leave/ }).click(),
  ]);
}

const legendLabels = (page: Page) =>
  page.locator("#legend .chip").evaluateAll((chips) => chips.map((chip) => (chip.firstChild?.textContent ?? "").trim()));

function lastMark(deckId: string, cardId: string): string | undefined {
  const file = path.join(WORKSPACE, ".alix", "progress", `${deckId}.json`);
  const progress = JSON.parse(fs.readFileSync(file, "utf8"));
  const history = progress.cards[cardId].history;
  return history[history.length - 1].confidence;
}

test("a tracked flip card asks Sure or Unsure in place of Reveal and records the mark", async ({ page }) => {
  await start(page, "Flip", "Recall");
  await page.getByRole("button", { name: "Reveal" }).click();
  await page.getByRole("button", { name: "Seen" }).click();
  await expect(page.getByText("session complete", { exact: true })).toBeVisible();
  await leave(page);

  await start(page, "Flip", "Recall", { cram: true, track: true });
  expect(await legendLabels(page), "the bottom row before the answer").toEqual(["Unsure", "Sure", "Hint"]);
  await expect(page.locator("#legend .chip", { hasText: /^Hint/ }), "no hint: shown, disabled").toBeDisabled();
  await expect(page.getByRole("button", { name: /^Unsure/ }), "both are primary").toHaveClass(/primary/);
  const unsureBox = (await page.getByRole("button", { name: /^Unsure/ }).boundingBox())!;
  const sureBox = (await page.getByRole("button", { name: /^Sure/ }).boundingBox())!;
  const axis = (unsureBox.x + unsureBox.width + sureBox.x) / 2;
  const width = page.viewportSize()!.width;
  expect(Math.abs(axis - width / 2), `the gap between Unsure and Sure sits on the centre (${axis} vs ${width / 2})`).toBeLessThan(2);
  const seam = sureBox.x - (unsureBox.x + unsureBox.width);
  expect(Math.abs(seam), `Unsure and Sure meet as one split button (seam ${seam}px)`).toBeLessThan(1);
  const unsureRadius = await page.getByRole("button", { name: /^Unsure/ }).evaluate((b) => getComputedStyle(b).borderTopRightRadius);
  expect(unsureRadius, "the seam side of Unsure is square").toBe("0px");
  await page.screenshot({ path: path.join(__dirname, "..", ".tmp", "confidence-row.png"), clip: { x: 0, y: unsureBox.y - 20, width, height: unsureBox.height + 40 } });
  const [mark] = await Promise.all([
    page.waitForRequest((request) => request.url().includes("/api/confidence")),
    page.getByRole("button", { name: /^Sure/ }).click(),
  ]);
  expect(mark.postDataJSON()).toEqual({ card: "card-confflip0001", confidence: "sure" });
  await expect(page.getByText("Oslo"), "Sure reveals the answer").toBeVisible();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/grade")),
    page.getByRole("button", { name: /^Missed it/ }).click(),
  ]);
  expect(lastMark("deck-00000000000000000000000071", "card-confflip0001")).toBe("sure");

  await leave(page);
  await start(page, "Flip", "Recall", { cram: true });
  expect(await legendLabels(page), "the deck remembers tracking").toEqual(["Unsure", "Sure", "Hint"]);
  await expect(page.getByRole("button", { name: /^Unsure/ }).locator(".k")).toHaveText("space");
  await expect(page.getByRole("button", { name: /^Sure/ }).locator(".k")).toHaveText("enter");
  const [byKey] = await Promise.all([
    page.waitForRequest((request) => request.url().includes("/api/confidence")),
    page.keyboard.press("Space"),
  ]);
  expect(byKey.postDataJSON().confidence, "Space is Unsure on a flip card").toBe("unsure");
  await expect(page.getByText("Oslo"), "Space reveals the answer").toBeVisible();
});

test("a tracked single-choice pick waits for Sure or Unsure before it is submitted", async ({ page }) => {
  await start(page, "Pick", "Recognize");
  await page.locator(".options .option").first().click();
  await page.getByRole("button", { name: /^Seen/ }).click();
  await leave(page);

  await start(page, "Pick", "Recognize", { cram: true, track: true });
  const chooses: string[] = [];
  page.on("request", (request) => { if (request.url().includes("/api/choose")) chooses.push(request.postData() ?? ""); });
  await expect(page.getByRole("button", { name: /^Sure/ }), "nothing picked yet").toBeDisabled();
  const stockholm = page.locator(".options .option", { hasText: "Stockholm" });
  await stockholm.click();
  await expect(stockholm, "the pick is shown as selected").toHaveClass(/selected/);
  expect(chooses, "a pick alone submits nothing").toEqual([]);
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/choose")),
    page.getByRole("button", { name: /^Unsure/ }).click(),
  ]);
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/grade")),
    page.getByRole("button", { name: /^Next/ }).click(),
  ]);
  expect(lastMark("deck-00000000000000000000000072", "card-confpick0001")).toBe("unsure");
});

test("skip intro grades a never-seen card at once instead of introducing it", async ({ page }) => {
  const introduces: string[] = [];
  page.on("request", (request) => { if (request.url().includes("/api/introduce")) introduces.push(request.url()); });
  const [select] = await Promise.all([
    page.waitForRequest((request) => request.url().includes("/api/select")),
    start(page, "Flip", "Recall", { skipIntro: true }),
  ]);
  expect(select.postDataJSON().skip_introduction, "the launch carries the tick-box").toBe(true);
  await page.getByRole("button", { name: /^Reveal/ }).click();
  await expect(page.getByRole("button", { name: /^Seen/ }), "no introduction").toHaveCount(0);
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/grade")),
    page.getByRole("button", { name: /^Got it/ }).click(),
  ]);
  expect(introduces, "nothing was introduced").toEqual([]);
});

test("a tracked multi-line typed answer takes Shift+Space as Unsure", async ({ page }) => {
  await start(page, "Pair", "Reconstruct", { skipIntro: true, track: true });
  await expect(page.getByRole("button", { name: /^Unsure/ }).locator(".k")).toHaveText("shift+space");
  await expect(page.getByRole("button", { name: /^Sure/ }).locator(".k")).toHaveText("shift+enter");
  await page.locator("textarea").fill("Oslo\nStockholm");
  const [mark] = await Promise.all([
    page.waitForRequest((request) => request.url().includes("/api/confidence")),
    page.keyboard.press("Shift+Space"),
  ]);
  expect(mark.postDataJSON().confidence).toBe("unsure");
  await expect(page.getByText("Stockholm", { exact: true }).first(), "the answer is revealed").toBeVisible();
});

test("several typed blanks: Enter moves to the next field, Shift+Enter submits", async ({ page }) => {
  await start(page, "Blanks", "Reconstruct", { skipIntro: true });
  const fields = page.locator("#card .blank-field");
  await expect(fields).toHaveCount(2);
  const checks: string[] = [];
  page.on("request", (request) => { if (request.url().includes("/api/check")) checks.push(request.url()); });
  await fields.nth(0).fill("Oslo");
  await page.keyboard.press("Enter");
  await expect(fields.nth(1), "Enter moves on").toBeFocused();
  await fields.nth(1).fill("Stockholm");
  await page.keyboard.press("Enter");
  expect(checks, "Enter on the last field submits nothing").toEqual([]);
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/check")),
    page.keyboard.press("Shift+Enter"),
  ]);
});
