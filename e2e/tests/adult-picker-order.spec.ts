// A real-server regression for numbered deck order in the adult picker. The
// temporary workspace reproduces the reported `10, 100, 11` sequence without
// changing the shared frozen fixtures, then cleans itself even after failure.
import fs from "node:fs";
import path from "node:path";
import { test, expect } from "./helpers";
import { adultDeckRow, openApp } from "./helpers";

const NUMBERED_WORKSPACE = path.join(
  __dirname,
  "..",
  ".tmp",
  "adult",
  "decks",
  "numbered-order",
);

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await openApp(page);
});

test.afterEach(() => {
  fs.rmSync(NUMBERED_WORKSPACE, { recursive: true, force: true });
});

test("the picker orders numbered deck titles naturally", async ({ page }) => {
  fs.mkdirSync(path.join(NUMBERED_WORKSPACE, "decks"), { recursive: true });
  fs.writeFileSync(path.join(NUMBERED_WORKSPACE, "alix.toml"), 'title = "Numbered Order"\n');
  for (const [file, id, cardId, title] of [
    [
      "10. PDF-Native Sourcing.md",
      "deck-00000000000000000000000010",
      "card-numbered10",
      "10. PDF-Native Sourcing",
    ],
    [
      "100. Private, Reviewable Bug Reports.md",
      "deck-00000000000000000000000100",
      "card-numbered100",
      "100. Private, Reviewable Bug Reports",
    ],
    [
      "11. Blocking Frontend Static Analysis.md",
      "deck-00000000000000000000000011",
      "card-numbered11",
      "11. Blocking Frontend Static Analysis",
    ],
  ]) {
    fs.writeFileSync(
      path.join(NUMBERED_WORKSPACE, "decks", file),
      `---\nformat-version: 1\nid: "${id}"\ntitle: "${title}"\n---\n## Question\nAnswer\n<!-- id: ${cardId} -->\n`,
    );
  }

  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Numbered Order").click();

  await expect(page.locator(".deckrow .name")).toHaveText([
    "10. PDF-Native Sourcing",
    "11. Blocking Frontend Static Analysis",
    "100. Private, Reviewable Bug Reports",
  ]);
  await expect(page.locator(".deckrow .badge-new", { hasText: "new" })).toHaveCount(3);
});
