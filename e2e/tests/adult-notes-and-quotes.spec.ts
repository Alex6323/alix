// A real-server regression for the blockquote redesign in the adult client:
// a bare blockquote is quoted answer content with its `>` markers gone, and
// several badged blockquotes stack as separate notes in authored order. The
// temporary workspace keeps the exact path independent of the shared
// fixtures, then removes it even when an assertion fails.
import fs from "node:fs";
import path from "node:path";
import { test, expect } from "./helpers";
import { adultDeckRow, openApp } from "./helpers";

const NOTES_WORKSPACE = path.join(
  __dirname,
  "..",
  ".tmp",
  "adult",
  "decks",
  "notes-and-quotes",
);

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await openApp(page);
});

test.afterEach(async ({ request }) => {
  await request.post("/api/deselect", { data: {} });
  fs.rmSync(NOTES_WORKSPACE, { recursive: true, force: true });
});

test("a quotation is answer content and badged notes stack", async ({ page }) => {
  fs.mkdirSync(path.join(NOTES_WORKSPACE, "decks"), { recursive: true });
  fs.writeFileSync(path.join(NOTES_WORKSPACE, "alix.toml"), 'title = "Notes And Quotes"\n');
  fs.writeFileSync(
    path.join(NOTES_WORKSPACE, "decks", "dijkstra.md"),
    `---
format-version: 1
id: "deck-00000000000000000000000014"
title: "Dijkstra"
---
## What did Dijkstra say about testing?
That it shows the presence of bugs, never their absence.
> Program testing can be used to show the presence of bugs, but never
> to show their absence.

> [!NOTE]
> From "The Humble Programmer", 1972.

> [!WARNING]
> It is not a claim that testing is useless.
<!-- id: card-quotenote1 -->
`,
  );

  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Notes And Quotes").click();
  await adultDeckRow(page, "Dijkstra").click();
  await page.getByTitle("choose a depth").click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: /^Recall/ }).click(),
  ]);

  await expect(page.locator(".front-text")).toHaveText(
    "What did Dijkstra say about testing?",
  );
  await page.getByRole("button", { name: "Reveal" }).click();

  // The quotation belongs to the answer, joined as prose joins anywhere else,
  // and no `>` survives into what the learner reads.
  const quote = page.locator("#ansRegion blockquote.quote");
  await expect(quote).toHaveCount(1);
  await expect(quote).toHaveText(
    "Program testing can be used to show the presence of bugs, but never to show their absence.",
  );
  await expect(page.locator("#ansRegion")).not.toContainText(">");

  // Two badged blockquotes are two notes, in authored order, each carrying its
  // own badge rather than one overwriting the other.
  const notes = page.locator(".note");
  await expect(notes).toHaveCount(2);
  await expect(notes.nth(0)).toHaveAttribute("data-badge", "note");
  await expect(notes.nth(0)).toContainText('From "The Humble Programmer", 1972.');
  await expect(notes.nth(1)).toHaveAttribute("data-badge", "warning");
  await expect(notes.nth(1)).toContainText("It is not a claim that testing is useless.");
});

// Line reveal walks the answer's STEPS, so a two-line quotation is one
// reveal action and arrives as a block, not as two `>` lines.
test("a quotation reveals as one block under `reveal: line`", async ({ page }) => {
  fs.mkdirSync(path.join(NOTES_WORKSPACE, "decks"), { recursive: true });
  fs.writeFileSync(path.join(NOTES_WORKSPACE, "alix.toml"), 'title = "Notes And Quotes"\n');
  fs.writeFileSync(
    path.join(NOTES_WORKSPACE, "decks", "dijkstra.md"),
    `---
format-version: 1
id: "deck-00000000000000000000000014"
title: "Dijkstra"
---
## What did Dijkstra say about testing?
That it shows the presence of bugs, never their absence.
> Program testing can be used to show the presence of bugs, but never
> to show their absence.
The point is about what a passing suite cannot prove.
<!-- reveal: line -->
<!-- id: card-quoteline1 -->
`,
  );

  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Notes And Quotes").click();
  await adultDeckRow(page, "Dijkstra").click();
  await page.getByTitle("choose a depth").click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: /^Recall/ }).click(),
  ]);

  const revealed = page.locator("#ansRegion .reveal.line > .answer:not(.pending):not(.line-reserve)");
  // The unrevealed tail renders as `.line-reserve` to hold the layout, so a
  // revealed quotation is the one that is NOT reserve.
  const quote = page.locator("#ansRegion blockquote.quote:not(.line-reserve)");

  await page.getByRole("button", { name: "Reveal" }).click();
  await expect(revealed).toHaveCount(1);
  await expect(quote).toHaveCount(0);

  // ONE more reveal takes the whole two-line quotation, as a block.
  await page.getByRole("button", { name: "Reveal" }).click();
  await expect(quote).toHaveCount(1);
  await expect(quote).toHaveText(
    "Program testing can be used to show the presence of bugs, but never to show their absence.",
  );
  await expect(page.locator("#ansRegion")).not.toContainText(">");

  await page.getByRole("button", { name: "Reveal" }).click();
  await expect(revealed).toHaveCount(2);
});
