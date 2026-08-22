// A real-server regression for a new ordered card in the adult client. The
// temporary workspace keeps the exact first-encounter path independent of the
// shared fixtures, then removes it even when an assertion fails.
import fs from "node:fs";
import path from "node:path";
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
## The boot sequence, in order <!-- id: card-lineintro1 --> <!-- reveal: line -->
Firmware runs from ROM.
The bootloader loads the kernel.
The kernel starts init.
Init brings up services.
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

  const answerLines = page.locator(".reveal .answer:not(.pending)");
  await expect(answerLines).toHaveText(["Firmware runs from ROM."]);
  await expect(page.getByRole("button", { name: "Reveal next" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Seen" })).toHaveCount(0);

  await page.keyboard.press("Space");
  await expect(answerLines).toHaveText([
    "Firmware runs from ROM.",
    "The bootloader loads the kernel.",
  ]);
  await page.getByRole("button", { name: "Reveal next" }).click();
  await page.getByRole("button", { name: "Reveal next" }).click();

  await expect(answerLines).toHaveText([
    "Firmware runs from ROM.",
    "The bootloader loads the kernel.",
    "The kernel starts init.",
    "Init brings up services.",
  ]);
  await expect(page.getByRole("button", { name: "Seen" })).toBeVisible();
});
