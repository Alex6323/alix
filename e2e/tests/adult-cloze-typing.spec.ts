// At Reconstruct a cloze card's blanks are its typing fields, and the result
// shows in the blanks. The temporary workspace keeps these cards independent
// of the shared fixtures, then removes it even when an assertion fails.
import fs from "node:fs";
import path from "node:path";
import type { Page } from "@playwright/test";
import { test, expect } from "./helpers";
import { adultDeckRow, openApp } from "./helpers";

const WORKSPACE = path.join(__dirname, "..", ".tmp", "adult", "decks", "cloze-typing");

function deck(id: string, title: string, body: string): string {
  return `---\nformat-version: 1\nid: "${id}"\ntitle: "${title}"\n---\n${body}`;
}

// The directives run against reading order: "cache" is read first but asked
// second, so a field's position in the page is not its answer's position.
function against(n: number): string {
  return deck(`deck-0000000000000000000000004${n}`, `Memory Pair ${n}`, `## Memory pair
the cache sits beside the lunate
<!-- blank: span [pair] hidden="lunate" b:e2c10${n} -->
<!-- blank: span [pair] hidden="cache" b:e2c11${n} -->
<!-- id: card-clozetyped${n} -->
`);
}

const FENCED = deck("deck-00000000000000000000000048", "Fenced Pair", `## Fenced pair
\`\`\`text
the cache sits beside the lunate
\`\`\`
<!-- blank: span [pair] hidden="lunate" b:e2c103 -->
<!-- blank: span [pair] hidden="cache" b:e2c104 -->
<!-- id: card-clozefenced -->
`);

async function openTyped(page: Page, file: string, text: string, title: string): Promise<void> {
  fs.mkdirSync(path.join(WORKSPACE, "decks"), { recursive: true });
  fs.writeFileSync(path.join(WORKSPACE, "alix.toml"), 'title = "Cloze Typing"\n');
  fs.writeFileSync(path.join(WORKSPACE, "decks", file), text);
  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Cloze Typing").click();
  await adultDeckRow(page, title).click();
  // A first encounter is introduced, never tested; the cram pass after it is
  // the real Reconstruct check.
  await page.getByTitle("choose a depth").click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: /^Reconstruct/ }).click(),
  ]);
  await page.getByRole("button", { name: "Reveal" }).click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/introduce")),
    page.getByRole("button", { name: "Seen" }).click(),
  ]);
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/deselect")),
    page.getByRole("button", { name: /^Leave/ }).click(),
  ]);
  await adultDeckRow(page, title).click();
  await page.getByTitle("choose a depth").click();
  await page.getByRole("button", { name: /cram/i }).click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: /^Reconstruct/ }).click(),
  ]);
}

async function typeAndSubmit(page: Page, values: string[]): Promise<string[]> {
  const blanks = page.locator("#card .region.q input.blank-field");
  await expect(blanks, "one field per asked blank").toHaveCount(values.length);
  await expect(blanks.first(), "the first blank has focus").toBeFocused();
  for (const [index, value] of values.entries()) {
    await blanks.nth(index).focus();
    await page.keyboard.type(value);
  }
  const [request] = await Promise.all([
    page.waitForRequest((req) => req.url().includes("/api/check")),
    page.keyboard.press("Enter"),
  ]);
  await expect(page.locator("#card .region.q .blank-filled"), "the blanks fill after the check").toHaveCount(values.length);
  return request.postDataJSON().lines;
}

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await openApp(page);
});

test.afterEach(async ({ request }) => {
  await request.post("/api/deselect", { data: {} });
  fs.rmSync(WORKSPACE, { recursive: true, force: true });
});

test("blanks typed in place are checked by position even against directive order", async ({ page }, testInfo) => {
  await openTyped(page, "against6.md", against(6), "Memory Pair 6");
  await expect(page.locator("#ansRegion input.field"), "no field below the question").toHaveCount(0);
  await expect(page.locator("#card .region.q + .divider"), "no hairline on a cloze card").toBeHidden();
  await page.locator("#card").screenshot({ path: testInfo.outputPath("typed-before.png") });

  const lines = await typeAndSubmit(page, ["cache", "lunate"]);
  expect(lines, "lines go out in directive order").toEqual(["lunate", "cache"]);
  const filled = page.locator("#card .region.q .blank-filled");
  await expect(filled, "both blanks pass").toHaveClass([/pass/, /pass/]);
  await expect(filled, "the blanks show the answers in reading order").toHaveText(["cache", "lunate"]);
  await expect(page.locator("#card .blank-typed"), "nothing is struck").toHaveCount(0);
  await expect(page.locator("#ansRegion .reveal"), "no checked list below the question").toHaveCount(0);
  await page.locator("#card").screenshot({ path: testInfo.outputPath("typed-right.png") });
});

test("a blank typed wrong shows the typed text struck before the answer", async ({ page }, testInfo) => {
  await openTyped(page, "against7.md", against(7), "Memory Pair 7");
  const lines = await typeAndSubmit(page, ["lunate", "cache"]);
  expect(lines, "swapped values stay at their blanks' positions").toEqual(["cache", "lunate"]);
  const filled = page.locator("#card .region.q .blank-filled");
  await expect(filled, "both blanks miss").toHaveClass([/miss/, /miss/]);
  await expect(page.locator("#card .region.q .blank-typed"), "each miss shows what was typed there").toHaveText(["lunate", "cache"]);
  await page.locator("#card").screenshot({ path: testInfo.outputPath("typed-wrong.png") });
});

test("blanks inside a fence keep their own fields below the question", async ({ page }) => {
  await openTyped(page, "fenced.md", FENCED, "Fenced Pair");
  await expect(page.locator("#ansRegion input.field"), "one field per blank below the question").toHaveCount(2);
  await expect(page.locator("#card input.blank-field"), "no field in the fence").toHaveCount(0);
});

test("browse drops the hairline on a cloze card and keeps it on a plain one", async ({ page }) => {
  fs.mkdirSync(path.join(WORKSPACE, "decks"), { recursive: true });
  fs.writeFileSync(path.join(WORKSPACE, "alix.toml"), 'title = "Cloze Typing"\n');
  fs.writeFileSync(path.join(WORKSPACE, "decks", "mixed.md"), deck("deck-00000000000000000000000049", "Mixed", `## Memory pair
the cache sits beside the lunate
<!-- blank: span hidden="cache" b:e2c121 -->
<!-- id: card-clozebrowse1 -->

## What sits in front of main memory?
cache
<!-- id: card-clozebrowse2 -->
`));
  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Cloze Typing").click();
  await adultDeckRow(page, "Mixed").hover();
  await page.keyboard.press("b");
  const hairline = page.locator(".card .region.q + .divider");
  await expect(page.locator(".card .region.q"), "browse opens on the cloze card").toContainText("Memory pair");
  await expect(hairline, "the cloze card has no hairline").toBeHidden();
  await page.getByRole("button", { name: "Next" }).click();
  await expect(page.locator(".card .region.q"), "browse moves to the plain card").toContainText("main memory");
  await expect(hairline, "the plain card keeps its hairline").toBeVisible();
});
