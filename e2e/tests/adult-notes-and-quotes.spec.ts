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

  // Styled, not merely tagged: each note leads with a chip naming its badge,
  // the two badges tint their boxes differently, and the quotation carries a
  // rule rather than a `>`.
  await expect(notes.nth(0).locator(".note-badge")).toHaveText(/note/i);
  await expect(notes.nth(1).locator(".note-badge")).toHaveText(/warning/i);
  const tints = await notes.evaluateAll((boxes) =>
    boxes.map((box) => getComputedStyle(box).backgroundColor),
  );
  expect(new Set(tints).size).toBe(2);
  await expect(quote).toHaveCSS("border-left-width", "3px");

  // The accent paints borders and washes, never small text: measured across
  // the 21 palettes, accent-on-its-own-wash falls to 2.0:1. Each chip takes
  // the note's own ink and keeps the hue in its border.
  for (const index of [0, 1]) {
    const chip = notes.nth(index).locator(".note-badge");
    const [ink, border, body] = await chip.evaluate((node) => [
      getComputedStyle(node).color,
      getComputedStyle(node).borderTopColor,
      getComputedStyle(node.parentElement as HTMLElement).color,
    ]);
    expect(ink).toBe(body);
    expect(ink).not.toBe(border);
  }
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

// A table answer walks the same reference card. Codex found the tutor's
// `ui` missing `appendTable` after the table step landed, which throws on
// the step walk rather than degrading, so the panel never opens.
test("the tutor reference shows a table as a table", async ({ page }) => {
  fs.mkdirSync(path.join(NOTES_WORKSPACE, "decks"), { recursive: true });
  fs.writeFileSync(path.join(NOTES_WORKSPACE, "alix.toml"), 'title = "Notes And Quotes"\n');
  fs.writeFileSync(
    path.join(NOTES_WORKSPACE, "decks", "ports.md"),
    `---
format-version: 1
id: "deck-00000000000000000000000015"
title: "Ports"
---
## Which ports do these protocols use?
The well-known assignments:
| protocol | port |
| --- | --- |
| http | 80 |
| https | 443 |
<!-- id: card-tabletutor1 -->
`,
  );

  await page.route("**/api/ask", (route) =>
    route.fulfill({
      json: {
        transcript: [{ q: "Why?", a: "Because the source says so." }],
        thinking: false,
        status: null,
        error: null,
        draft: null,
      },
    }),
  );

  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Notes And Quotes").click();
  await adultDeckRow(page, "Ports").click();
  await page.getByTitle("choose a depth").click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: /^Recall/ }).click(),
  ]);
  await page.getByRole("button", { name: "Reveal" }).click();
  await page.getByRole("button", { name: "Ask tutor" }).click();

  const table = page.locator(".ask-card table");
  await expect(table).toHaveCount(1);
  await expect(table).toContainText("443");
  await expect(page.locator(".ask-card")).not.toContainText("|");
});

// The tutor's reference card shows the learner what they were asked, so a
// quotation must read as one there too. Found while sweeping for the
// `back.length` derivations Codex's kids finding pointed at.
test("the tutor reference shows a quotation as quoted content", async ({ page }) => {
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
<!-- id: card-quotetutor1 -->
`,
  );

  await page.route("**/api/ask", (route) =>
    route.fulfill({
      json: {
        transcript: [{ q: "Why?", a: "Because the source says so." }],
        thinking: false,
        status: null,
        error: null,
        draft: null,
      },
    }),
  );

  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Notes And Quotes").click();
  await adultDeckRow(page, "Dijkstra").click();
  await page.getByTitle("choose a depth").click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: /^Recall/ }).click(),
  ]);
  await page.getByRole("button", { name: "Reveal" }).click();
  await page.getByRole("button", { name: "Ask tutor" }).click();

  const quote = page.locator(".ask-card blockquote.quote");
  await expect(quote).toHaveCount(1);
  await expect(page.locator(".ask-card")).not.toContainText(">");
});
