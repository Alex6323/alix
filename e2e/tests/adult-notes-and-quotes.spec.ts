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

  // Styled, not merely tagged: each note leads with its badge word, the bar
  // beside it takes the same colour, the two badges differ, and the
  // quotation carries a rule rather than a `>`. Nothing between the answer
  // and the note: the bar and the word are the separation.
  await expect(notes.nth(0).locator(".note-badge")).toHaveText(/note/i);
  await expect(notes.nth(1).locator(".note-badge")).toHaveText(/warning/i);
  const accents = await notes.evaluateAll((boxes) =>
    boxes.map((box) => [
      getComputedStyle(box).borderLeftColor,
      getComputedStyle(box.querySelector(".note-badge")!).color,
    ]),
  );
  for (const [bar, word] of accents) expect(bar).toBe(word);
  expect(new Set(accents.map(([bar]) => bar)).size).toBe(2);
  await expect(page.locator("#noteDivider")).toHaveCount(0);
  await expect(quote).toHaveCSS("border-left-width", "3px");

  // Prose below the question is set ragged right at every width: the answer,
  // the quotation and the notes never justify, on a wide desktop or a phone
  // held upright alike.
  const sentences = page.locator(
    "#ansRegion .reveal > .answer, #ansRegion blockquote.quote > .answer, .note > p",
  );
  await expect(sentences).toHaveCount(4);
  for (const viewport of [900, 450, 360]) {
    await page.setViewportSize({ width: viewport, height: 800 });
    await expect
      .poll(
        () => sentences.evaluateAll((nodes) => nodes.map((node) => getComputedStyle(node).textAlign)),
        { message: `answer, quotation, two notes at a ${viewport}px viewport` },
      )
      .toEqual(["start", "start", "start", "start"]);
  }
});

// GitHub's five badge colours, one set per brightness: every badge on a dark
// palette paints the dark set on its bar and word, and the light set on a
// light palette, whatever the palette's own accents are.
test("every badge takes GitHub's colour for the palette's brightness", async ({ page }) => {
  fs.mkdirSync(path.join(NOTES_WORKSPACE, "decks"), { recursive: true });
  fs.writeFileSync(path.join(NOTES_WORKSPACE, "alix.toml"), 'title = "Five Badges"\n');
  fs.writeFileSync(
    path.join(NOTES_WORKSPACE, "decks", "badges.md"),
    `---
format-version: 1
id: "deck-00000000000000000000000017"
title: "Five Badges"
---
## Which badges exist?
Five.
> [!NOTE]
> A note.

> [!TIP]
> A tip.

> [!IMPORTANT]
> An important one.

> [!WARNING]
> A warning.

> [!CAUTION]
> A caution.
<!-- id: card-fivebadges00000001 -->
`,
  );
  const sets: Record<string, Record<string, string>> = {
    "ayu-dark": { note: "#4493f8", tip: "#3fb950", important: "#ab7df8", warning: "#d29922", caution: "#f85149" },
    "solarized-light": { note: "#0969da", tip: "#1a7f37", important: "#8250df", warning: "#9a6700", caution: "#cf222e" },
  };
  const rgb = (hex: string) =>
    `rgb(${[1, 3, 5].map((index) => Number.parseInt(hex.slice(index, index + 2), 16)).join(", ")})`;

  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Five Badges").click();
  await adultDeckRow(page, "badges").click();
  await page.getByTitle("choose a depth").click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: /^Recall/ }).click(),
  ]);
  await page.getByRole("button", { name: "Reveal" }).click();
  await expect(page.locator(".note")).toHaveCount(5);

  for (const [palette, colours] of Object.entries(sets)) {
    await page.evaluate((id) => {
      document.documentElement.dataset.theme = id;
    }, palette);
    const painted = await page.locator(".note").evaluateAll((boxes) =>
      boxes.map((box) => [
        (box as HTMLElement).dataset.badge,
        getComputedStyle(box).borderLeftColor,
        getComputedStyle(box.querySelector(".note-badge")!).color,
      ]),
    );
    for (const [badge, bar, word] of painted) {
      expect(bar, `${palette}: ${badge} bar`).toBe(rgb(colours[badge!]));
      expect(word, `${palette}: ${badge} word`).toBe(rgb(colours[badge!]));
    }
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
