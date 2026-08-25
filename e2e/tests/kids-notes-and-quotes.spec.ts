// The kids half of the blockquote redesign. The temporary workspace keeps the
// exact path independent of the shared fixtures, then removes it even when an
// assertion fails.
//
// Kids deliberately flattens the notes into one mascot bubble and ignores the
// badge (`renderWhy`, kids/study.js); badge styling and stacked callouts are
// increment 5's work, so nothing here asserts them.
import fs from "node:fs";
import path from "node:path";
import { test, expect } from "./helpers";
import { kidsDeckRow, openApp } from "./helpers";

const KIDS_NOTES_WORKSPACE = path.join(
  __dirname,
  "..",
  ".tmp",
  "kids",
  "decks",
  "notes-and-quotes",
);

const DECK = `---
format-version: 1
id: "deck-00000000000000000000000015"
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
<!-- id: card-kidsquotenote1 -->
`;

const ONE_LINE_DECK = `---
format-version: 1
id: "deck-00000000000000000000000016"
title: "Aphorism"
---
## What is the one rule?
> Never cross the streams.
<!-- id: card-kidsonelinequote -->
`;

function writeWorkspace(deck: string = DECK, name = "dijkstra.md"): void {
  fs.mkdirSync(path.join(KIDS_NOTES_WORKSPACE, "decks"), { recursive: true });
  fs.writeFileSync(
    path.join(KIDS_NOTES_WORKSPACE, "alix.toml"),
    'title = "Notes And Quotes"\n',
  );
  fs.writeFileSync(path.join(KIDS_NOTES_WORKSPACE, "decks", name), deck);
}

async function revealTheAnswer(
  page: Parameters<typeof openApp>[0],
  deckTitle = "Dijkstra",
): Promise<void> {
  await openApp(page);
  await page.locator(".box", { hasText: "Notes And Quotes" }).click();
  await kidsDeckRow(page, deckTitle).click();
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/select")),
    page.getByRole("button", { name: "Say it yourself" }).click(),
  ]);
  await page.getByRole("button", { name: "Show me" }).click();
}

test.afterEach(async ({ request }) => {
  await request.post("/api/deselect", { data: {} });
  fs.rmSync(KIDS_NOTES_WORKSPACE, { recursive: true, force: true });
});

test("every badged note is spoken, in authored order", async ({ page }) => {
  writeWorkspace();
  await revealTheAnswer(page);

  await expect(page.locator(".rev-why-text p")).toHaveText([
    'From "The Humble Programmer", 1972.',
    "It is not a claim that testing is useless.",
  ]);
});

test("a quotation renders as quoted content, not as `>` lines", async ({ page }) => {
  writeWorkspace();
  await revealTheAnswer(page);

  const quote = page.locator("#stage blockquote.quote");
  await expect(quote).toHaveCount(1);
  await expect(quote).toHaveText(
    "Program testing can be used to show the presence of bugs, but never to show their absence.",
  );
});

// The shortest quotation is the common one (a definition, a warning, an
// aphorism), and it is the one that used to slip through: the render path was
// chosen from `back.length`, so a single quoted line took the plain-text
// branch and the child read the `>` marker. Found by Codex.
test("a one-line quotation is quoted content too", async ({ page }) => {
  writeWorkspace(ONE_LINE_DECK, "aphorism.md");
  await revealTheAnswer(page, "Aphorism");

  const quote = page.locator("#stage blockquote.quote");
  await expect(quote).toHaveCount(1);
  await expect(quote).toHaveText("Never cross the streams.");
  await expect(page.locator("#stage")).not.toContainText(">");
});
