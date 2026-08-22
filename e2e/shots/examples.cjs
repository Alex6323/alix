#!/usr/bin/env node
"use strict";
/*
 * Example-deck screenshots: one image per deck in docs/examples/, showing
 * what that deck's syntax produces on screen.
 *
 * Standalone and manual, like its neighbour capture.cjs, and for the same
 * reasons: it needs a browser and `cwebp`, neither of which CI has. The CI
 * half is scripts/check-example-media.py, which proves every example has an
 * image and every image belongs to an example. Neither proves an image is
 * CURRENT, and no byte comparison can: a choice card's options are shuffled
 * from a seed reseeded every session, so those decks photograph differently
 * on every run. Re-running and reading the picture is what proves it.
 *
 *   node e2e/shots/examples.cjs [--only=table,cloze]
 *
 * Unlike capture.cjs this serves the committed example decks themselves,
 * copied into a scratch directory so a capture never writes review state
 * into the repository.
 */
const { chromium } = require("@playwright/test");
const { execFileSync, spawn } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const REPO_ROOT = path.join(__dirname, "..", "..");
const EXAMPLES = path.join(REPO_ROOT, "docs", "examples");
const WORK = path.join(__dirname, ".tmp-examples");
const PORT = 7899;

function log(...parts) {
  process.stderr.write(`${parts.join(" ")}\n`);
}

// Photograph the checkout, not whatever `alix` happens to be installed: an
// older binary treats a new frontmatter key as unknown and silently
// photographs the fallback.
function buildAlix() {
  log("building the checkout (cargo build --release)");
  execFileSync("cargo", ["build", "--release", "--quiet"], {
    cwd: REPO_ROOT,
    stdio: ["ignore", "inherit", "inherit"],
  });
  return path.join(REPO_ROOT, "target", "release", "alix");
}

function requireCwebp() {
  try {
    execFileSync("cwebp", ["-version"], { stdio: "ignore" });
  } catch {
    throw new Error(
      "cwebp is required (Debian/Ubuntu: apt install webp; macOS: brew install webp)",
    );
  }
}

/// Every example deck, as {set, name, source}. The sets are the two the
/// examples README describes; a new set is picked up without editing this.
function examples() {
  const found = [];
  for (const set of ["shapes", "syntax"]) {
    const dir = path.join(EXAMPLES, set);
    if (!fs.existsSync(dir)) continue;
    for (const file of fs.readdirSync(dir).sort()) {
      if (file.endsWith(".md")) {
        found.push({ set, name: path.basename(file, ".md"), source: path.join(dir, file) });
      }
    }
  }
  return found;
}

/// The header logo plays a ~2.6s reveal on load, so a fixed pause photographs
/// it mid-division. Shoot until two consecutive frames are identical instead:
/// that settles any animation, not just the one we know about.
async function settled(page) {
  let previous = null;
  for (let attempt = 0; attempt < 60; attempt += 1) {
    const shot = await page.screenshot({ type: "png" });
    if (previous && shot.equals(previous)) return shot;
    previous = shot;
    await page.waitForTimeout(150);
  }
  throw new Error("the page never stopped animating");
}

/// Compare an option to a deck line without caring how it was written: a
/// formula reaches the page as KaTeX, whose TeX carries no `$` delimiters.
function normalize(text) {
  return text.replace(/\$/g, "").replace(/\s+/g, " ").trim();
}

/// The authored answers of a deck: one per `- [x]` line.
function authoredAnswers(source) {
  return source
    .split("\n")
    .filter((line) => line.trimStart().startsWith("- [x]"))
    .map((line) => normalize(line.trimStart().slice(5)));
}

/// Every card-table row, as its list of cells. A table card's answer is the
/// question's own row partner, not an authored mark, so this is how a table
/// example is answered.
function tableRows(source) {
  return source
    .split("\n")
    .filter((line) => line.trimStart().startsWith("|"))
    .map((line) =>
      line
        .replace(/<!--.*?-->/g, "")
        .split("|")
        .map((cell) => normalize(cell))
        .filter(Boolean),
    )
    .filter((cells) => cells.length > 1 && !cells.every((cell) => /^-+$/.test(cell)));
}

/// What an option says. A formula is rendered to SVG paths, which carry no
/// readable text, so fall back to the TeX kept on the run's aria-label.
async function optionText(option) {
  const text = (await option.locator(".opt").innerText()).trim();
  if (text) return normalize(text);
  const math = option.locator(".opt .math-run").first();
  if ((await math.count()) === 0) return "";
  return normalize((await math.getAttribute("aria-label")) || "");
}

/// Which on-screen option is the answer. Authored `[x]` decides it when the
/// card carries one; otherwise the question and its answer are cells of the
/// same table row, and the distractors are drawn from other rows, so exactly
/// one option shares a row with the question.
function correctOption(question, options, source) {
  const authored = authoredAnswers(source);
  const marked = options.find((option) => authored.includes(option));
  if (marked) return marked;
  for (const cells of tableRows(source)) {
    if (!cells.includes(question)) continue;
    const partner = options.find((option) => cells.includes(option));
    if (partner) return partner;
  }
  return null;
}

/// Answer an on-screen choice card, because the cursor rests on option 1 and
/// a photograph of that reads as if option 1 were the answer. Matching on
/// text rather than position is what survives the option shuffle, which is
/// reseeded every session on purpose.
async function answerChoice(page, source) {
  const options = page.locator(".options .option");
  const count = await options.count();
  if (count === 0) return;
  const texts = [];
  for (let i = 0; i < count; i += 1) {
    texts.push(await optionText(options.nth(i)));
  }
  const question = await questionText(page);
  const answer = correctOption(question, texts, source);
  if (answer === null) {
    throw new Error(`no option answers ${question}: ${texts.join(" | ")}`);
  }
  await options.nth(texts.indexOf(answer)).click();
  await page.locator(".option.correct").waitFor({ state: "visible", timeout: 5_000 });
}

/// Reveal the answer of a `reveal: line` deck. A fresh deck's cards are all
/// new, and `reveal: line` becomes line-by-line only at Recall (see
/// depth::check_for), so the front photographs a plain card. Revealing at
/// least shows the steps the shape will later uncover one at a time, and
/// the line count is asserted: an example whose answer collapsed to a
/// single line fails here rather than shipping a picture of a shape it
/// cannot demonstrate.
async function revealSteps(page) {
  await page.keyboard.press("Space");
  const answer = page.locator(".reveal").first();
  await answer.waitFor({ state: "visible", timeout: 5_000 });
  const lines = (await answer.innerText())
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
  if (lines.length < 2) {
    throw new Error(`a reveal: line example needs a multi-line answer, got ${lines.length}`);
  }
}

/// The question as it reads on screen: the last line of the question region,
/// since a card table puts its container's title above the term.
async function questionText(page) {
  return normalize((await page.locator(".region.q").innerText()).split("\n").pop());
}

/// Hand-drawn answers, keyed by the question they answer, as strokes in a
/// unit square. An empty canvas photographs as an empty box, so the draw
/// example has to carry ink for the shape to read as a drawing surface at
/// all. Keyed rather than generic because a scribble under "draw the
/// hiragana for ka" would be a wrong answer in a picture that teaches.
const SKETCHES = {
  'Draw the hiragana for "ka".': [
    [
      [0.24, 0.32],
      [0.42, 0.27],
      [0.62, 0.26],
      [0.66, 0.36],
      [0.63, 0.54],
      [0.53, 0.71],
      [0.4, 0.82],
      [0.31, 0.84],
      [0.28, 0.75],
    ],
    [
      [0.42, 0.1],
      [0.37, 0.32],
      [0.3, 0.56],
      [0.19, 0.86],
    ],
    [
      [0.79, 0.33],
      [0.83, 0.53],
    ],
  ],
};

/// Draw the card's answer on its canvas. The strokes are proportioned in a
/// square, so they are mapped into one centred in the canvas rather than
/// stretched across it.
async function sketchAnswer(page, question) {
  const canvas = page.locator(".card canvas").first();
  if ((await canvas.count()) === 0) return;
  const strokes = SKETCHES[question];
  if (!strokes) throw new Error(`no sketch for a draw card asking: ${question}`);
  const box = await canvas.boundingBox();
  const side = Math.min(box.width, box.height) * 0.95;
  const left = box.x + (box.width - side) / 2;
  const top = box.y + (box.height - side) / 2;
  for (const stroke of strokes) {
    const points = stroke.map(([x, y]) => [left + x * side, top + y * side]);
    await page.mouse.move(points[0][0], points[0][1]);
    await page.mouse.down();
    for (const [x, y] of points.slice(1)) await page.mouse.move(x, y, { steps: 12 });
    await page.mouse.up();
  }
}

async function waitForServer(base) {
  for (let attempt = 0; attempt < 60; attempt += 1) {
    try {
      const response = await fetch(`${base}/api/version`);
      if (response.ok) return;
    } catch {
      // not up yet
    }
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
  throw new Error(`server at ${base} never came up`);
}

async function main() {
  requireCwebp();
  const only = (process.argv.find((a) => a.startsWith("--only=")) || "").slice(7);
  const wanted = only ? new Set(only.split(",")) : null;
  const decks = examples().filter((d) => !wanted || wanted.has(d.name));
  if (decks.length === 0) throw new Error("no example decks matched");

  // Serve copies, never the repository: a review writes progress beside the
  // deck it drills, and an example deck must stay exactly as committed.
  fs.rmSync(WORK, { recursive: true, force: true });
  fs.mkdirSync(WORK, { recursive: true });
  for (const deck of decks) {
    fs.copyFileSync(deck.source, path.join(WORK, `${deck.name}.md`));
  }

  const server = spawn(buildAlix(), [WORK, "--port", String(PORT), "--session", "20"], {
    cwd: REPO_ROOT,
    stdio: ["ignore", "pipe", "pipe"],
  });
  server.stdout.on("data", (d) => process.stderr.write(`[alix] ${d}`));
  const stop = () => {
    try {
      server.kill("SIGTERM");
    } catch {
      // already gone
    }
  };
  process.on("exit", stop);

  const base = `http://127.0.0.1:${PORT}`;
  await waitForServer(base);
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1000, height: 720 } });

  try {
    for (const deck of decks) {
      const response = await fetch(`${base}/api/select`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ deck: `${deck.name}.md` }),
      });
      if (!response.ok) throw new Error(`select failed for ${deck.name}: ${response.status}`);
      await page.goto(base, { waitUntil: "networkidle" });
      await page.locator(".card").first().waitFor({ state: "visible", timeout: 10_000 });
      const source = fs.readFileSync(deck.source, "utf8");
      await answerChoice(page, source);
      await sketchAnswer(page, await questionText(page));
      if (/^reveal:\s*line\s*$/m.test(source)) await revealSteps(page);

      const out = path.join(EXAMPLES, deck.set, `${deck.name}.webp`);
      const png = path.join(WORK, `${deck.name}.png`);
      try {
        fs.writeFileSync(png, await settled(page));
        execFileSync("cwebp", ["-quiet", "-lossless", "-z", "9", png, "-o", out]);
      } finally {
        fs.rmSync(png, { force: true });
      }
      log("wrote", path.relative(REPO_ROOT, out));
    }
  } finally {
    await browser.close();
    stop();
  }
}

main().catch((error) => {
  log(`examples: ${error.message}`);
  process.exit(1);
});
