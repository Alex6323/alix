// A cloze card's question fills in place on reveal in the adult client. The
// temporary workspace keeps these cards independent of the shared fixtures,
// then removes it even when an assertion fails.
import fs from "node:fs";
import path from "node:path";
import type { Page } from "@playwright/test";
import { test, expect } from "./helpers";
import { adultDeckRow, openApp } from "./helpers";

const WORKSPACE = path.join(__dirname, "..", ".tmp", "adult", "decks", "cloze-reveal");
const HIDDEN = ["lunate", "hamate"];

function deck(id: string, title: string, frontmatter: string, body: string): string {
  return `---\nformat-version: 1\nid: "${id}"\ntitle: "${title}"\n${frontmatter}---\n${body}`;
}

const SIBLINGS = deck("deck-00000000000000000000000041", "Carpal Siblings", "", `## Carpal bones
the lunate sits beside the hamate
<!-- blank: span hidden="lunate" b:e2c001 -->
<!-- blank: span hidden="hamate" b:e2c002 -->
<!-- id: card-clozesiblings -->
`);

const PAIR = deck("deck-00000000000000000000000042", "Carpal Pair", "reveal: line\n", `## Carpal pair
the lunate sits beside the hamate
<!-- blank: span [pair] hidden="lunate" b:e2c003 -->
<!-- blank: span [pair] hidden="hamate" b:e2c004 -->
<!-- id: card-clozepair -->
`);

const FENCE = deck("deck-00000000000000000000000043", "Carpal Fence", "", `## Fence heading
the lunate sits here
<!-- blank: span hidden="lunate" b:e2c005 -->
\`\`\`text
x
\`\`\`
<!-- id: card-clozefence -->
`);

async function openDeck(page: Page, file: string, text: string, title: string): Promise<void> {
  fs.mkdirSync(path.join(WORKSPACE, "decks"), { recursive: true });
  fs.writeFileSync(path.join(WORKSPACE, "alix.toml"), 'title = "Cloze Reveal"\n');
  fs.writeFileSync(path.join(WORKSPACE, "decks", file), text);
  await page.locator("#navRefresh").click();
  await adultDeckRow(page, "Cloze Reveal").click();
  await adultDeckRow(page, title).click();
  await page.getByTitle("choose a depth").click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: /^Recall/ }).click(),
  ]);
}

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await openApp(page);
});

test.afterEach(async ({ request }) => {
  await request.post("/api/deselect", { data: {} });
  fs.rmSync(WORKSPACE, { recursive: true, force: true });
});

async function accentColor(page: Page): Promise<string> {
  return page.evaluate(() => {
    const probe = document.createElement("span");
    probe.style.color = "var(--bolt-hi)";
    document.body.appendChild(probe);
    const color = getComputedStyle(probe).color;
    probe.remove();
    return color;
  });
}

test("a cloze card shows no hidden text before reveal", async ({ page }, testInfo) => {
  await openDeck(page, "siblings.md", SIBLINGS, "Carpal Siblings");
  const question = page.locator("#card .region.q");
  const asked = question.locator(".blank-pill:not(.muted)");
  const muted = question.locator(".blank-pill.muted");

  await expect(question).toContainText("Carpal bones");
  const before = await question.innerText();
  for (const word of HIDDEN) expect(before, `(a) hidden word ${word} before reveal`).not.toContain(word);
  await expect(page.locator("#ansRegion .reveal"), "(a) no answer section before reveal").toHaveCount(0);
  await expect(asked, "(a) one asked pill").toHaveCount(1);
  await expect(muted, "(a) one muted sibling pill").toHaveCount(1);
  await expect(asked.locator(".blank-glyph"), "(a) the asked marker glyph is not drawn").toBeHidden();
  await expect(muted.locator(".blank-glyph"), "(a) the hidden marker glyph is not drawn").toBeHidden();
  await expect(asked, "(a) the asked pill is outlined in the accent").toHaveCSS("border-top-color", await accentColor(page));

  const askedWidth = (await asked.boundingBox())?.width;
  const mutedWidth = (await muted.boundingBox())?.width;
  expect(askedWidth, "(b) the asked pill has a width").toBeGreaterThan(0);
  expect(mutedWidth, "(b) asked and muted pills share one width").toBe(askedWidth);

  await page.locator("#card").screenshot({ path: testInfo.outputPath("adult-dark-before.png") });
  await page.evaluate(() => { document.documentElement.dataset.theme = "light"; });
  await page.locator("#card").screenshot({ path: testInfo.outputPath("adult-light-before.png") });
});

test("reveal fills a cloze card's blank in place, highlighted, without an answer repeat", async ({ page }, testInfo) => {
  await openDeck(page, "siblings.md", SIBLINGS, "Carpal Siblings");
  const question = page.locator("#card .region.q");
  const answer = page.locator("#ansRegion");
  const asked = question.locator(".blank-pill:not(.muted)");
  const muted = question.locator(".blank-pill.muted");
  const filled = question.locator(".blank-filled");

  await expect(question).toContainText("Carpal bones");
  await page.getByRole("button", { name: "Reveal" }).click();
  await expect(filled, "(c) one filled blank").toHaveCount(1);
  const word = await filled.innerText();
  expect(HIDDEN, `(c) the filled word ${word} is a hidden word`).toContain(word);
  await expect(filled, "(c) the filled word wears the accent").toHaveCSS("color", await accentColor(page));
  await expect(asked, "(c) no asked pill remains").toHaveCount(0);
  await expect(muted, "(c) the sibling stays hidden").toHaveCount(1);
  const sibling = HIDDEN.find((other) => other !== word) as string;
  expect(await question.innerText(), `(c) sibling ${sibling} stays hidden`).not.toContain(sibling);
  expect(await answer.innerText(), `(c) the answer area does not repeat ${word}`).not.toContain(word);

  await page.locator("#card").screenshot({ path: testInfo.outputPath("adult-dark-after.png") });
  await page.evaluate(() => { document.documentElement.dataset.theme = "light"; });
  await page.locator("#card").screenshot({ path: testInfo.outputPath("adult-light-after.png") });
  await page.evaluate(() => { document.documentElement.dataset.theme = "dark"; });

  await page.keyboard.press("h");
  await expect(filled, "(c) the self-test conceal re-masks the question").toHaveCount(0);
  await expect(asked, "(c) the asked pill is back while concealed").toHaveCount(1);
  await page.keyboard.press("h");
  await expect(filled, "(c) showing again fills the blank").toHaveCount(1);
});

test("line reveal fills one own blank per press", async ({ page }, testInfo) => {
  await openDeck(page, "pair.md", PAIR, "Carpal Pair");
  const question = page.locator("#card .region.q");
  const asked = question.locator(".blank-pill:not(.muted)");
  const filled = question.locator(".blank-filled");

  await expect(asked, "(d) two asked pills before reveal").toHaveCount(2);
  await page.getByRole("button", { name: "Reveal" }).click();
  await expect(filled, "(d) the first press fills the first blank").toHaveText(["lunate"]);
  await expect(asked, "(d) the second blank is still asked").toHaveCount(1);
  await expect(page.getByRole("button", { name: "Reveal next" })).toBeVisible();
  await page.locator("#card").screenshot({ path: testInfo.outputPath("adult-dark-line-step1.png") });

  await page.getByRole("button", { name: "Reveal next" }).click();
  await expect(filled, "(d) the second press fills both").toHaveText(["lunate", "hamate"]);
  await expect(asked, "(d) no asked pill remains").toHaveCount(0);
  await expect(page.getByRole("button", { name: "Seen" })).toBeVisible();
});

test("a cloze question's heading stays its own line beside a code fence", async ({ page }) => {
  await openDeck(page, "fence.md", FENCE, "Carpal Fence");
  const lines = page.locator("#card .region.q .front-line");
  await expect(lines.first(), "(e) the heading is its own line").toHaveText("Fence heading");
  await expect(page.locator("#card .region.q pre .blank-pill"), "(e) no pill inside the fence").toHaveCount(0);
  await expect(page.locator("#card .region.q .front-line .blank-pill"), "(e) the prose pill").toHaveCount(1);
});
