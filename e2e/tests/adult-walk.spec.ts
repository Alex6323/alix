import fs from "node:fs";
import path from "node:path";
import { test, expect, adultDeckRow, openApp } from "./helpers";
import type { Page } from "@playwright/test";

const WALK_WORKSPACE = path.join(__dirname, "..", ".tmp", "adult", "decks", "walk-tour");

const CHOICE_FRONT = "Which animal has eight arms?";
const FLIP_FRONT = "What does a hermit crab live in?";

const DECK = `---
id: "deck-000000000000000000000000w1"
title: Tide pools
---

# Tide pools

Rock pools hold water when the tide goes out.

## ${CHOICE_FRONT}
- [x] Octopus
- [ ] Starfish
- [ ] Crab
<!-- choices: single -->
<!-- id: card-walkchoice -->

## ${FLIP_FRONT}
A borrowed shell
<!-- id: card-walkflip -->
`;

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  fs.rmSync(WALK_WORKSPACE, { recursive: true, force: true });
  fs.mkdirSync(path.join(WALK_WORKSPACE, "decks"), { recursive: true });
  fs.writeFileSync(path.join(WALK_WORKSPACE, "alix.toml"), 'title = "Walk Tour"\n');
  fs.writeFileSync(path.join(WALK_WORKSPACE, "decks", "tide.md"), DECK);
  await openApp(page);
});

test.afterEach(async ({ request }) => {
  await request.post("/api/deselect", { data: {} });
  fs.rmSync(WALK_WORKSPACE, { recursive: true, force: true });
});

async function openWalkMenu(page: Page) {
  await adultDeckRow(page, "Tide pools").click();
  await page.getByRole("button", { name: /^Depth…/ }).click();
  return page.locator(".legend .chip.walk");
}

async function closeTopicSheet(page: Page, step: string) {
  await expect(page.locator(".topic-drawer"), `${step}: the topic sheet opens on first contact`).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".topic-drawer"), `${step}: Escape closes the sheet`).toHaveCount(0);
}

test("a walk serves the unwalked rest, then a full pass, with the topic sheet each walk", async ({ page }) => {
  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Walk Tour").click();

  // Walk 1: the never-walked items in deck order, so the choice item leads.
  const walk = await openWalkMenu(page);
  await expect(walk.locator("span"), "fresh deck: no hint").toHaveCount(1);
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk") && r.request().method() === "POST" && r.ok()),
    walk.click(),
  ]);
  await closeTopicSheet(page, "walk 1");
  await expect(page.locator(".front-text")).toHaveText(CHOICE_FRONT);
  await expect(page.locator("#hist")).toHaveText("2 left");
  const octopus = page.locator(".option").filter({ has: page.locator(".opt", { hasText: "Octopus", exact: true }) });
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/choose") && r.ok()),
    octopus.click(),
  ]);
  await expect(octopus, "walk 1 choice: the pick shows right or wrong").toHaveClass(/correct/);
  await expect(page.getByRole("button", { name: /^Ask/ })).toBeVisible();
  await expect(page.getByRole("button", { name: /^I guessed/ }), "walk 1 correct pick: the quiet guess is offered").toBeVisible();
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/grade") && r.ok()),
    page.getByRole("button", { name: /^Next/ }).click(),
  ]);
  await expect(page.locator(".front-text")).toHaveText(FLIP_FRONT);

  // Leaving mid-walk keeps what was walked: one item is still new.
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/leave") && r.ok()),
    page.keyboard.press("Escape"),
  ]);
  await expect(adultDeckRow(page, "Tide pools")).toBeVisible();
  const again = await openWalkMenu(page);
  await expect(again.locator("span"), "after walking the choice item: no hint").toHaveCount(1);

  // Walk 2 holds only the never-walked flip item and shows the sheet again.
  await again.click();
  await closeTopicSheet(page, "walk 2");
  await expect(page.locator(".topic-title"), "walk 2: no heading on the card face").toHaveCount(0);
  await page.getByRole("button", { name: /^Context/ }).click();
  await closeTopicSheet(page, "walk 2 Context chip");
  await expect(page.locator(".front-text")).toHaveText(FLIP_FRONT);
  await expect(page.locator("#hist"), "walk 2 counts only the unwalked item").toHaveText("1 left");

  // A reload mid-walk resumes from GET /api/walk.
  await page.reload({ waitUntil: "domcontentloaded" });
  await expect(page.locator(".front-text"), "reload resumes the walk").toHaveText(FLIP_FRONT);
  if (await page.locator(".topic-drawer").count()) await page.keyboard.press("Escape");
  await expect(page.locator(".topic-drawer")).toHaveCount(0);
  await expect(page.locator("#ansRegion .reveal"), "flip front hides the answer").toHaveCount(0);
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/reveal") && r.ok()),
    page.keyboard.press(" "),
  ]);
  await expect(page.locator("#ansRegion .reveal")).toContainText("A borrowed shell");
  await expect(page.getByRole("button", { name: /^Missed it/ }), "flip answer: the three grades").toBeVisible();
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/grade") && r.ok()),
    page.keyboard.press("3"),
  ]);

  // Done: Next walk replaces Next session and starts walk 3, a full pass.
  await expect(page.locator(".summary .lede"), "walk 2 ends after its one item").toHaveText("walk complete");
  await expect(page.getByRole("button", { name: /^Next session/ })).toHaveCount(0);
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/restart") && r.ok()),
    page.getByRole("button", { name: /^Next walk/ }).click(),
  ]);
  await closeTopicSheet(page, "walk 3");
  await expect(page.locator(".front-text"), "walk 3 starts at the least recently walked item").toHaveText(CHOICE_FRONT);
  await expect(page.locator("#hist"), "walk 3 is a full pass").toHaveText("2 left");
  await page.keyboard.press("1");
  await expect(octopus).toHaveClass(/correct/);

  await page.keyboard.press("Escape");
  await adultDeckRow(page, "Walk Tour").click();
  const walked = await openWalkMenu(page);
  await expect(walked).toBeVisible();
  await expect(walked.locator("span"), "every item walked: no hint").toHaveCount(1);
});

test("a cram walk starts after the last walked item and serves every item", async ({ page }) => {
  await adultDeckRow(page, "Walk Tour").click();
  const walk = await openWalkMenu(page);
  await walk.click();
  await closeTopicSheet(page, "first walk");
  const octopus = page.locator(".option").filter({ has: page.locator(".opt", { hasText: "Octopus", exact: true }) });
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/choose") && r.ok()),
    octopus.click(),
  ]);
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/grade") && r.ok()),
    page.getByRole("button", { name: /^Next/ }).click(),
  ]);
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/leave") && r.ok()),
    page.keyboard.press("Escape"),
  ]);

  const cramWalk = await openWalkMenu(page);
  await page.getByRole("button", { name: /cram/i }).click();
  const [opened] = await Promise.all([
    page.waitForRequest((r) => r.url().endsWith("/api/walk") && r.method() === "POST"),
    cramWalk.click(),
  ]);
  expect(opened.postDataJSON(), "cram walk: the request asks for cram").toMatchObject({ cram: true });
  await closeTopicSheet(page, "cram walk");
  await expect(page.locator(".front-text"), "cram walk: starts after the walked choice item").toHaveText(FLIP_FRONT);
  await expect(page.locator("#hist"), "cram walk: serves both items").toHaveText("2 left");

  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/leave") && r.ok()),
    page.keyboard.press("Escape"),
  ]);
  await adultDeckRow(page, "Tide pools").click();
  const [plain] = await Promise.all([
    page.waitForRequest((r) => r.url().endsWith("/api/walk") && r.method() === "POST"),
    page.locator(".legend .chip.primary", { hasText: /^Walk/ }).click(),
  ]);
  expect(plain.postDataJSON(), "a later primary Walk is plain").toMatchObject({ cram: false });
});

test("the tutor on an ordinary walk offers Make a note and Make a card", async ({ page }) => {
  await adultDeckRow(page, "Walk Tour").click();
  const walk = await openWalkMenu(page);
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk") && r.request().method() === "POST" && r.ok()),
    walk.click(),
  ]);
  await closeTopicSheet(page, "ordinary walk");
  const octopus = page.locator(".option").filter({ has: page.locator(".opt", { hasText: "Octopus", exact: true }) });
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/choose") && r.ok()),
    octopus.click(),
  ]);
  await page.getByRole("button", { name: /^Ask/ }).click();
  await expect(page.locator(".ask-panel")).toBeVisible();
  await expect(page.locator("#legend .chip.distill", { hasText: /^Make this a note/ }), "ordinary walk: Make a note").toHaveCount(1);
  await expect(page.locator("#legend .chip.distill", { hasText: /^Make this a card/ }), "ordinary walk: Make a card").toHaveCount(1);
  await page.keyboard.press("Escape");
});

test("a wrong pick survives a reload with only Continue, and the next walk serves the unwalked item before the miss", async ({ page }) => {
  await adultDeckRow(page, "Walk Tour").click();
  const walk = await openWalkMenu(page);
  await walk.click();
  await closeTopicSheet(page, "wrong pick");
  const starfish = page.locator(".option").filter({ has: page.locator(".opt", { hasText: "Starfish", exact: true }) });
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/choose") && r.ok()),
    starfish.click(),
  ]);
  await page.reload({ waitUntil: "domcontentloaded" });
  if (await page.locator(".topic-drawer").count()) await page.keyboard.press("Escape");
  await expect(page.getByRole("button", { name: /^Continue/ }), "reload after a wrong pick: Continue").toBeVisible();
  await expect(page.getByRole("button", { name: /^(Next|I guessed|Got it)/ }), "reload after a wrong pick: no pass").toHaveCount(0);
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/grade") && r.ok()),
    page.getByRole("button", { name: /^Continue/ }).click(),
  ]);
  await page.keyboard.press("Escape");
  await adultDeckRow(page, "Walk Tour").click();
  const again = await openWalkMenu(page);
  await again.click();
  await closeTopicSheet(page, "after the miss");
  await expect(page.locator(".front-text"), "the never-walked flip item leads").toHaveText(FLIP_FRONT);
  await expect(page.locator("#hist"), "the missed choice item still follows").toHaveText("2 left");
});

const HINTED_FRONT = "Where does a sea anemone hold on?";

const HINT_DECK = `---
id: "deck-000000000000000000000000w1"
title: Tide pools
---

# Tide pools

Rock pools hold water when the tide goes out.

## ${FLIP_FRONT}
A borrowed shell
<!-- id: card-walkflip -->

## ${HINTED_FRONT}
> [!TIP]
> Something hard under the water.

---
On a rock
<!-- id: card-walkhinted -->
`;

test("a walk opens a hint on the question, and a hinted Got it leaves the card waiting", async ({ page }) => {
  fs.writeFileSync(path.join(WALK_WORKSPACE, "decks", "tide.md"), HINT_DECK);
  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Walk Tour").click();
  const walk = await openWalkMenu(page);
  await walk.click();
  await closeTopicSheet(page, "hint walk");
  const hint = page.locator("#legend .chip", { hasText: /^Hint/ });
  await expect(page.locator(".front-text")).toHaveText(FLIP_FRONT);
  await expect(hint, "a card without hints offers no Hint chip").toHaveCount(0);
  await page.keyboard.press(" ");
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/grade") && r.ok()),
    page.getByRole("button", { name: /^Got it/ }).click(),
  ]);

  await expect(page.locator(".front-text")).toHaveText(HINTED_FRONT);
  await expect(hint, "a hinted question offers the Hint chip").toBeVisible();
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/hint") && r.status() === 204),
    page.keyboard.press("i"),
  ]);
  await expect(page.locator(".topic-drawer .hint-stack .note"), "the hint opens").toContainText("Something hard under the water.");
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: /^Reveal/ }).click();
  await expect(hint, "the answer offers no hint").toHaveCount(0);
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/grade") && r.ok()),
    page.getByRole("button", { name: /^Got it/ }).click(),
  ]);

  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/restart") && r.ok()),
    page.getByRole("button", { name: /^Next walk/ }).click(),
  ]);
  await closeTopicSheet(page, "after the hinted Got it");
  await expect(page.locator(".front-text"), "the hinted card waits").toHaveText(HINTED_FRONT);
  await expect(page.locator("#hist"), "only the hinted card waits").toHaveText("1 left");
});

const LINE_DECK = `---
id: "deck-000000000000000000000000w1"
title: Tide pools
---

# Tide pools

Rock pools hold water when the tide goes out.

## How does a tide pool form, step by step?
The tide rises over the rocks
The tide falls again
Water stays behind in the hollows
<!-- reveal: line -->
<!-- id: card-walklines -->
`;

test("a walk reveals a reveal: line card one line at a time", async ({ page }) => {
  fs.writeFileSync(path.join(WALK_WORKSPACE, "decks", "tide.md"), LINE_DECK);
  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Walk Tour").click();
  const walk = await openWalkMenu(page);
  await walk.click();
  await closeTopicSheet(page, "line walk");
  const shown = page.locator("#ansRegion .reveal.line .answer:not(.pending):not(.line-reserve)");
  const reveals: string[] = [];
  page.on("request", (r) => { if (r.url().endsWith("/api/walk/reveal")) reveals.push(r.url()); });

  await page.keyboard.press(" ");
  await expect(shown, "first press: one line").toHaveCount(1);
  await expect(page.getByRole("button", { name: /^Reveal next/ }), "first press: more lines to come").toBeVisible();
  await page.keyboard.press(" ");
  await expect(shown, "second press: two lines").toHaveCount(2);
  expect(reveals, "partial lines stay on the client").toHaveLength(0);

  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/reveal") && r.ok()),
    page.keyboard.press(" "),
  ]);
  await expect(shown, "last press: every line").toHaveCount(3);
  await expect(page.getByRole("button", { name: /^Got it/ }), "last line opens the answer and its grades").toBeVisible();
});

test("the topic sheet fits its content and closes from its own button", async ({ page }) => {
  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Walk Tour").click();
  await (await openWalkMenu(page)).click();
  const panel = page.locator(".topic-drawer-panel");
  await expect(panel, "open: the sheet shows on first contact").toBeVisible();
  await panel.evaluate((node) => Promise.all(node.getAnimations().map((motion) => motion.finished)));
  const box = (await panel.boundingBox())!;
  const viewport = page.viewportSize()!;
  expect(
    Math.abs(box.y + box.height - viewport.height),
    `open: the sheet rests on the window bottom (${viewport.height}px)`,
  ).toBeLessThanOrEqual(1);
  expect(box.y, `open: a one-sentence topic takes only the room it needs (${viewport.height}px)`).toBeGreaterThan(
    viewport.height / 2,
  );
  const closer = (await page.getByRole("button", { name: "Close topic context" }).boundingBox())!;
  expect(box.x + box.width - (closer.x + closer.width), "open: the close mark sits at the sheet's right edge").toBeLessThanOrEqual(24);
  expect(closer.y - box.y, "open: the close mark sits at the sheet's top edge").toBeLessThanOrEqual(24);
  await page.getByRole("button", { name: "Close topic context" }).click();
  await expect(page.locator(".topic-drawer"), "close: the mark removes the sheet").toHaveCount(0);

  const long = DECK.replace(
    "Rock pools hold water when the tide goes out.",
    Array.from({ length: 60 }, (_, i) => `Paragraph ${i + 1} about rock pools.\n`).join("\n"),
  );
  fs.writeFileSync(path.join(WALK_WORKSPACE, "decks", "tide.md"), long);
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/leave") && r.ok()),
    page.keyboard.press("Escape"),
  ]);
  await (await openWalkMenu(page)).click();
  await expect(panel, "long: the sheet shows").toBeVisible();
  await panel.evaluate((node) => Promise.all(node.getAnimations().map((motion) => motion.finished)));
  expect(
    await panel.evaluate((node) => node.scrollHeight > node.clientHeight),
    "long: the topic overflows, so the sheet scrolls",
  ).toBe(true);
  const tall = (await panel.boundingBox())!;
  expect(tall.y + tall.height, "long: the sheet stops at the window bottom").toBeLessThanOrEqual(viewport.height + 1);
  await expect(
    page.getByRole("button", { name: "Close topic context" }),
    "long: Close stays reachable",
  ).toBeInViewport();
});

test("at phone width the Depth menu footer chips never overlap", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.locator(".deckrow").first()).toBeFocused();
  await page.keyboard.press("r");
  await adultDeckRow(page, "Walk Tour").click();
  const walk = await openWalkMenu(page);
  await expect(walk).toBeVisible();

  const boxes = await page.locator(".legend .chip").evaluateAll((chips) =>
    chips.map((chip) => {
      const r = chip.getBoundingClientRect();
      return {
        label: (chip.textContent || "").trim(),
        left: r.left, right: r.right, top: r.top, bottom: r.bottom,
        wraps: r.height > 50,
      };
    }),
  );
  expect(boxes.length, `footer chips: ${boxes.map((b) => b.label).join(", ")}`).toBeGreaterThan(4);
  for (const box of boxes) {
    expect(box.wraps, `chip "${box.label}" keeps its label on one line`).toBe(false);
    expect(box.right, `chip "${box.label}" stays inside the viewport`).toBeLessThanOrEqual(390);
    expect(box.left, `chip "${box.label}" stays inside the viewport`).toBeGreaterThanOrEqual(0);
  }
  for (let i = 0; i < boxes.length; i++) {
    for (let j = i + 1; j < boxes.length; j++) {
      const a = boxes[i];
      const b = boxes[j];
      const overlap = a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
      expect(overlap, `"${a.label}" ${JSON.stringify(a)} and "${b.label}" ${JSON.stringify(b)} overlap`).toBe(false);
    }
  }
});
