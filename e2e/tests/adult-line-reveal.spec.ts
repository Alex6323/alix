// A real-server regression for a new ordered card in the adult client. The
// temporary workspace keeps the exact first-encounter path independent of the
// shared fixtures, then removes it even when an assertion fails.
import fs from "node:fs";
import path from "node:path";
import type { Page } from "@playwright/test";
import { test, expect } from "./helpers";
import { adultDeckRow, openApp } from "./helpers";

const LINE_WORKSPACE = path.join(
  __dirname,
  "..",
  ".tmp",
  "adult",
  "decks",
  "line-introduction",
);

async function firstAnswerTop(page: Page): Promise<number> {
  await page.locator(".reveal").evaluate(async (element) => {
    await Promise.all(element.getAnimations().map((animation) => animation.finished));
  });
  return page.locator(".reveal .answer:not(.pending):not(.line-reserve)").first().evaluate(
    (element) => element.getBoundingClientRect().top,
  );
}

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await openApp(page);
});

test.afterEach(async ({ request }) => {
  await request.post("/api/deselect", { data: {} });
  fs.rmSync(LINE_WORKSPACE, { recursive: true, force: true });
});

test("a new ordered card reveals one authored line at a time", async ({ page }) => {
  fs.mkdirSync(path.join(LINE_WORKSPACE, "decks"), { recursive: true });
  fs.writeFileSync(path.join(LINE_WORKSPACE, "alix.toml"), 'title = "Line Introduction"\n');
  fs.writeFileSync(
    path.join(LINE_WORKSPACE, "decks", "boot-sequence.md"),
    `---
format-version: 1
id: "deck-00000000000000000000000012"
title: "Boot Sequence"
---
## The boot sequence, in order
Firmware runs from ROM.
The bootloader loads the kernel.
The kernel starts init.
Init brings up services.
<!-- reveal: line -->
<!-- id: card-lineintro1 -->
`,
  );

  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Line Introduction").click();
  await adultDeckRow(page, "Boot Sequence").click();
  await page.getByTitle("choose a depth").click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: /^Recall/ }).click(),
  ]);

  await expect(page.locator(".front-text")).toHaveText("The boot sequence, in order");
  await page.getByRole("button", { name: "Reveal" }).click();

  const answerLines = page.locator(".reveal .answer:not(.pending):not(.line-reserve)");
  await expect(answerLines).toHaveText(["Firmware runs from ROM."]);
  await expect(page.getByRole("button", { name: "Reveal next" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Seen" })).toHaveCount(0);
  const firstTop = await firstAnswerTop(page);

  await page.keyboard.press("Space");
  await expect(answerLines).toHaveText([
    "Firmware runs from ROM.",
    "The bootloader loads the kernel.",
  ]);
  const secondTop = await firstAnswerTop(page);
  expect.soft(Math.abs(secondTop - firstTop), "the first revealed line moved").toBeLessThan(1);

  const continuation = page.locator(".answer.pending");
  await expect.soft(continuation).toHaveCSS("border-top-style", "solid");
  await expect.soft(continuation).toHaveCSS("border-radius", "999px");

  await page.getByRole("button", { name: "Reveal next" }).click();
  const thirdTop = await firstAnswerTop(page);
  expect.soft(Math.abs(thirdTop - firstTop), "the first revealed line moved").toBeLessThan(1);
  await page.getByRole("button", { name: "Reveal next" }).click();
  const fourthTop = await firstAnswerTop(page);
  expect.soft(Math.abs(fourthTop - firstTop), "the first revealed line moved").toBeLessThan(1);
  await expect(continuation).toBeHidden();

  await expect(answerLines).toHaveText([
    "Firmware runs from ROM.",
    "The bootloader loads the kernel.",
    "The kernel starts init.",
    "Init brings up services.",
  ]);
  await expect(page.getByRole("button", { name: "Seen" })).toBeVisible();
});
