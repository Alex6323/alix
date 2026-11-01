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

type Depth = "Recognize" | "Recall";

async function start(page: Page, deck: string, depth: Depth, opts: { cram?: boolean; track?: boolean } = {}) {
  await adultDeckRow(page, deck).click();
  await page.getByTitle("choose a depth").click();
  if (opts.cram) await page.getByRole("button", { name: /cram/i }).click();
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
  expect(await legendLabels(page), "the bottom row before the answer").toEqual(["Unsure", "Sure"]);
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
  expect(await legendLabels(page), "the deck remembers tracking").toEqual(["Unsure", "Sure"]);
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
