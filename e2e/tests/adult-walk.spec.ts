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

async function closeSectionSheet(page: Page, step: string) {
  await expect(page.locator(".section-drawer"), `${step}: the section sheet opens on first contact`).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".section-drawer"), `${step}: Escape closes the sheet`).toHaveCount(0);
}

test("a walk serves the unwalked rest, then a full pass, with the section sheet each walk", async ({ page }) => {
  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Walk Tour").click();

  // Walk 1: the never-walked items in deck order, so the choice item leads.
  const walk = await openWalkMenu(page);
  await expect(walk.locator("span"), "fresh deck: no hint").toHaveCount(1);
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk") && r.request().method() === "POST" && r.ok()),
    walk.click(),
  ]);
  await closeSectionSheet(page, "walk 1");
  await expect(page.locator(".front-text")).toHaveText(CHOICE_FRONT);
  await expect(page.locator("#hist")).toHaveText("2 left");
  const octopus = page.locator(".option").filter({ has: page.locator(".opt", { hasText: "Octopus", exact: true }) });
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/choose") && r.ok()),
    octopus.click(),
  ]);
  await expect(octopus, "walk 1 choice: the pick shows right or wrong").toHaveClass(/correct/);
  await expect(page.getByRole("button", { name: /^Ask tutor/ })).toBeVisible();
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/next") && r.ok()),
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
  await closeSectionSheet(page, "walk 2");
  await expect(page.locator(".front-text")).toHaveText(FLIP_FRONT);
  await expect(page.locator("#hist"), "walk 2 counts only the unwalked item").toHaveText("1 left");

  // A reload mid-walk resumes from GET /api/walk.
  await page.reload({ waitUntil: "domcontentloaded" });
  await expect(page.locator(".front-text"), "reload resumes the walk").toHaveText(FLIP_FRONT);
  if (await page.locator(".section-drawer").count()) await page.keyboard.press("Escape");
  await expect(page.locator(".section-drawer")).toHaveCount(0);
  await expect(page.locator("#ansRegion .reveal"), "flip front hides the answer").toHaveCount(0);
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/reveal") && r.ok()),
    page.keyboard.press(" "),
  ]);
  await expect(page.locator("#ansRegion .reveal")).toContainText("A borrowed shell");
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/next") && r.ok()),
    page.keyboard.press(" "),
  ]);

  // Done: Next walk replaces Next session and starts walk 3, a full pass.
  await expect(page.locator(".summary .lede"), "walk 2 ends after its one item").toHaveText("walk complete");
  await expect(page.getByRole("button", { name: /^Next session/ })).toHaveCount(0);
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/restart") && r.ok()),
    page.getByRole("button", { name: /^Next walk/ }).click(),
  ]);
  await closeSectionSheet(page, "walk 3");
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
