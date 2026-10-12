// A cloze card's question fills in place on reveal in the kids client. The
// temporary workspace keeps the card independent of the shared fixtures, then
// removes it even when an assertion fails.
import fs from "node:fs";
import path from "node:path";
import type { Page } from "@playwright/test";
import { test, expect } from "./helpers";
import { kidsDeckRow, openApp } from "./helpers";

const WORKSPACE = path.join(__dirname, "..", ".tmp", "kids", "decks", "cloze-reveal");
const HIDDEN = ["lunate", "hamate"];

const DECK = `---
format-version: 1
id: "deck-00000000000000000000000044"
title: "Carpal Siblings"
---
## Carpal bones
the lunate sits beside the hamate
<!-- blank: span hidden="lunate" b:e2c006 -->
<!-- blank: span hidden="hamate" b:e2c007 -->
<!-- id: card-kidsclozesiblings -->
`;

async function openDeck(page: Page): Promise<void> {
  fs.mkdirSync(path.join(WORKSPACE, "decks"), { recursive: true });
  fs.writeFileSync(path.join(WORKSPACE, "alix.toml"), 'title = "Cloze Reveal"\n');
  fs.writeFileSync(path.join(WORKSPACE, "decks", "siblings.md"), DECK);
  await openApp(page);
  await page.locator(".box", { hasText: "Cloze Reveal" }).click();
  await kidsDeckRow(page, "Carpal Siblings").click();
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/select")),
    page.getByRole("button", { name: "Say it yourself" }).click(),
  ]);
}

async function accentColor(page: Page): Promise<string> {
  return page.evaluate(() => {
    const probe = document.createElement("span");
    probe.style.color = "var(--accent)";
    document.body.appendChild(probe);
    const color = getComputedStyle(probe).color;
    probe.remove();
    return color;
  });
}

test.afterEach(async ({ request }) => {
  await request.post("/api/deselect", { data: {} });
  fs.rmSync(WORKSPACE, { recursive: true, force: true });
});

test("a kids cloze card shows no hidden text before reveal", async ({ page }, testInfo) => {
  await openDeck(page);
  const prompt = page.locator(".rev-prompt");
  await expect(prompt).toContainText("Carpal bones");
  const before = await page.locator(".rev-card").innerText();
  for (const word of HIDDEN) expect(before, `(a) hidden word ${word} before reveal`).not.toContain(word);
  await expect(prompt.locator(".blank-pill:not(.muted)"), "(a) one asked pill").toHaveCount(1);
  await expect(prompt.locator(".blank-pill.muted"), "(a) one muted sibling pill").toHaveCount(1);
  await expect(prompt.locator(".blank-pill:not(.muted)"), "(a) the asked pill is tinted").not.toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await expect(prompt.locator(".blank-pill.muted"), "(a) the sibling pill has a dashed outline").toHaveCSS("border-top-style", "dashed");
  await expect(prompt.locator(".blank-glyph").first(), "(a) the asked marker glyph is not drawn").toBeHidden();
  await expect(prompt.locator(".blank-glyph").last(), "(a) the hidden marker glyph is not drawn").toBeHidden();
  await expect(page.locator(".rev-blank"), "(a) no separate blank line under the question").toHaveCount(0);
  await page.locator(".rev-card").screenshot({ path: testInfo.outputPath("kids-before.png") });
});

test("a kids cloze reveal fills the blank in place, highlighted, without an answer repeat", async ({ page }, testInfo) => {
  await openDeck(page);
  const prompt = page.locator(".rev-prompt");
  await expect(prompt).toContainText("Carpal bones");
  await page.getByRole("button", { name: "Show me" }).click();
  const filled = prompt.locator(".blank-filled");
  await expect(filled, "(c) one filled blank").toHaveCount(1);
  const word = await filled.innerText();
  expect(HIDDEN, `(c) the filled word ${word} is a hidden word`).toContain(word);
  await expect(filled, "(c) the filled word wears the accent").toHaveCSS("color", await accentColor(page));
  await expect(page.locator(".rev-answer-fill"), "(c) no separate answer repeats it").toHaveCount(0);
  await expect(prompt.locator(".blank-pill.muted"), "(c) the sibling stays hidden").toHaveCount(1);
  await page.locator(".rev-card").screenshot({ path: testInfo.outputPath("kids-after.png") });
});
