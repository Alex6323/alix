import fs from "node:fs";
import path from "node:path";
import type { Page } from "@playwright/test";
import { test, expect, openApp } from "./helpers";

const TMP = path.join(__dirname, "..", ".tmp");
const EMPTY_SERVER = "http://127.0.0.1:7790";

function resetTutorial(): void {
  for (const server of ["adult", "adult-empty"]) {
    for (const dir of ["home", "state", "data"]) {
      fs.rmSync(path.join(TMP, server, dir), { recursive: true, force: true });
    }
  }
}

test.beforeAll(resetTutorial);

test.beforeEach(async ({ request }) => {
  await request.post("/api/deselect", { data: {} });
});

test.afterEach(async ({ request }) => {
  await request.post("/api/deselect", { data: {} });
});

async function closeSectionSheet(page: Page, step: string) {
  await expect(page.locator(".section-drawer"), `${step}: the section sheet is open`).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".section-drawer"), `${step}: Escape closes the sheet`).toHaveCount(0);
}

async function grade(page: Page, key: string, step: string) {
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/grade") && r.ok()),
    page.keyboard.press(key),
  ]);
  await expect(page.locator("#legend"), `${step}: graded`).toBeVisible();
}

async function reveal(page: Page) {
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/reveal") && r.ok()),
    page.keyboard.press(" "),
  ]);
}

async function openTutorialFromMenu(page: Page) {
  await page.locator("#kebab").click();
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/tutorial") && r.request().method() === "POST" && r.ok()),
    page.getByRole("menuitem", { name: "Tutorial" }).click(),
  ]);
  await closeSectionSheet(page, "tutorial open");
}

test("the header menu opens the tutorial walk and its cards teach by doing", async ({ page }) => {
  await openApp(page);
  await page.locator("#kebab").click();
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/tutorial") && r.request().method() === "POST" && r.ok()),
    page.getByRole("menuitem", { name: "Tutorial" }).click(),
  ]);
  await expect(page.locator(".section-drawer"), "first contact shows the section text").toContainText(
    "Welcome to Alix. This is a walk",
  );
  await closeSectionSheet(page, "tutorial open");
  await expect(page.locator("#hist")).toHaveText("9 left");

  await expect(page.locator(".front-text"), "card 1").toContainText("Reveal this answer, then end the card");
  await reveal(page);
  await expect(page.getByRole("button", { name: /^Missed it/ })).toBeVisible();
  await grade(page, "3", "card 1");

  await expect(page.locator(".front-text"), "card 2").toContainText("Open it again with the Context chip");
  await page.getByRole("button", { name: /^Context/ }).click();
  await closeSectionSheet(page, "card 2 Context chip");
  await reveal(page);
  await grade(page, "3", "card 2");

  await expect(page.locator(".front-text"), "card 3").toContainText("This card carries a hint");
  await page.locator("#legend .chip", { hasText: /^Hint/ }).click();
  await expect(page.locator(".hint-stack"), "card 3 hint").toContainText("Look at which of the three grades");
  await page.keyboard.press("Escape");
  await expect(page.locator(".hint-stack")).toHaveCount(0);
  await reveal(page);
  await grade(page, "3", "card 3");

  await expect(page.locator(".front-text"), "card 4").toContainText("which grade takes a card out");
  const pick = page.locator(".option").filter({ has: page.locator(".opt", { hasText: "Got it", exact: true }) });
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/choose") && r.ok()),
    pick.click(),
  ]);
  await expect(pick, "card 4: the right pick shows correct").toHaveClass(/correct/);
  await grade(page, "Enter", "card 4");

  await expect(page.locator(".front-text"), "card 5").toContainText("three steps of every card");
  const lines = page.locator("#ansRegion .reveal .answer:not(.pending):not(.line-reserve)");
  await page.keyboard.press(" ");
  await expect(lines, "card 5: one line so far").toHaveCount(1);
  await expect(lines.first()).toContainText("try to recall first");
  await page.keyboard.press(" ");
  await expect(lines, "card 5: two lines").toHaveCount(2);
  await reveal(page);
  await expect(lines, "card 5: all three lines").toHaveCount(3);
  await expect(lines.nth(2)).toContainText("then grade yourself");
  await grade(page, "3", "card 5");

  await expect(page.locator(".front-text"), "card 6").toContainText("A card you keep getting right");
  await expect(page.locator(".front-text"), "card 6: the blank hides the word").not.toContainText("forget");
  await reveal(page);
  await expect(page.locator(".front-text"), "card 6: the reveal fills the blank").toContainText("forget");
  await grade(page, "3", "card 6");

  await expect(page.locator(".front-text"), "card 7").toContainText("What is hiding for?");
  await reveal(page);
  const answer = page.locator("#ansRegion");
  await expect(answer).toContainText("A second try at recalling");
  await page.locator(".hide-toggle").click();
  await expect(answer, "card 7: hidden").toHaveClass(/concealed/);
  await page.locator(".hide-toggle").click();
  await expect(answer, "card 7: shown again").not.toHaveClass(/concealed/);
  await grade(page, "3", "card 7");

  await expect(page.locator(".front-text"), "card 8").toContainText("This card cites a file");
  await reveal(page);
  await page.locator(".cite-toggle").click();
  await expect(page.locator(".source-excerpt .source-text").first(), "card 8: the sample's first line").toContainText(
    "def greet(name, excited=False):",
  );
  await page.locator(".cite-toggle").click();
  await grade(page, "3", "card 8");

  await expect(page.locator(".front-text"), "card 9").toContainText("What is there beyond a walk?");
});

test("the tutor panel on a tutorial walk offers Ask but no note or card creation", async ({ page }) => {
  await openApp(page);
  await openTutorialFromMenu(page);
  await reveal(page);
  await page.getByRole("button", { name: /^Ask/ }).click();
  await expect(page.locator(".ask-panel")).toBeVisible();
  await expect(page.locator("#legend .chip", { hasText: /^Send/ })).toBeVisible();
  await expect(page.locator("#legend .chip.distill"), "no Make a note, no Make a card").toHaveCount(0);
  await page.keyboard.press("Escape");
});

test("leaving the tutorial lands on a picker that does not list it", async ({ page }) => {
  await openApp(page);
  await openTutorialFromMenu(page);
  await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/walk/leave") && r.ok()),
    page.keyboard.press("Escape"),
  ]);
  await expect(page.locator(".deckrow").first(), "the picker is back").toBeVisible();
  await expect(page.locator(".deckrow .name", { hasText: /^Tutorial$/ }), "the tutorial is not a library entry").toHaveCount(0);
});

test.describe("an empty decks folder", () => {
  test.use({ baseURL: EMPTY_SERVER });

  test.beforeEach(async ({ request }) => {
    await request.post("/api/deselect", { data: {} });
  });

  test("the picker offers a Tutorial button that opens the same walk", async ({ page }) => {
    await openApp(page);
    await expect(page.locator(".msg")).toContainText("No decks found");
    await Promise.all([
      page.waitForResponse((r) => r.url().endsWith("/api/tutorial") && r.request().method() === "POST" && r.ok()),
      page.getByRole("button", { name: "Tutorial" }).click(),
    ]);
    await expect(page.locator(".section-drawer")).toContainText("Welcome to Alix. This is a walk");
    await closeSectionSheet(page, "empty-picker tutorial");
    await expect(page.locator(".front-text")).toContainText("Reveal this answer, then end the card");
    await Promise.all([
      page.waitForResponse((r) => r.url().endsWith("/api/walk/leave") && r.ok()),
      page.keyboard.press("Escape"),
    ]);
    await expect(page.locator(".msg"), "leaving lands on the empty picker again").toContainText("No decks found");
  });
});
