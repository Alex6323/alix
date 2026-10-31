// Plain letters typed into an answer field are text, never shortcuts. The
// temporary workspace keeps the card independent of the shared fixtures, then
// removes it even when an assertion fails.
import fs from "node:fs";
import path from "node:path";
import { test, expect } from "./helpers";
import { adultDeckRow, openApp } from "./helpers";

const WORKSPACE = path.join(__dirname, "..", ".tmp", "adult", "decks", "typing-keys");

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await openApp(page);
});

test.afterEach(async ({ request }) => {
  await request.post("/api/deselect", { data: {} });
  fs.rmSync(WORKSPACE, { recursive: true, force: true });
});

test("typing the context key into an answer field types it and opens no section", async ({ page }) => {
  fs.mkdirSync(path.join(WORKSPACE, "decks"), { recursive: true });
  fs.writeFileSync(path.join(WORKSPACE, "alix.toml"), 'title = "Typing Keys"\n');
  fs.writeFileSync(
    path.join(WORKSPACE, "decks", "memory.md"),
    `---
format-version: 1
id: "deck-00000000000000000000000045"
title: "Memory"
---
# Memory tiers
A small fast store sits in front of a large slow one.

## What sits in front of main memory?
cache
<!-- id: card-typingkeys1 -->
`,
  );
  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Typing Keys").click();
  await adultDeckRow(page, "Memory").click();

  // A first encounter is introduced, never tested; the cram pass after it is
  // the real Reconstruct check.
  await page.getByTitle("choose a depth").click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: /^Reconstruct/ }).click(),
  ]);
  await page.keyboard.press("Escape");
  await expect(page.locator(".section-drawer"), "the first introduction's drawer closes").toHaveCount(0);
  await page.getByRole("button", { name: "Reveal" }).click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/introduce")),
    page.getByRole("button", { name: "Seen" }).click(),
  ]);
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/deselect")),
    page.getByRole("button", { name: /^Leave/ }).click(),
  ]);
  await adultDeckRow(page, "Memory").click();
  await page.getByTitle("choose a depth").click();
  await page.getByRole("button", { name: /cram/i }).click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: /^Reconstruct/ }).click(),
  ]);

  const field = page.locator("#ansRegion input.field");
  await expect(field, "the typed check offers one field").toHaveCount(1);
  await field.focus();
  await page.keyboard.type("cache");
  await expect(field, "every letter lands in the field").toHaveValue("cache");
  await expect(page.locator(".section-drawer"), "no section drawer opened").toBeHidden();
});
