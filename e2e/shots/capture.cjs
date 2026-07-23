#!/usr/bin/env node
"use strict";
/*
 * Landing-page carousel screenshot capture — alix.study.
 *
 * Standalone. NOT part of `make e2e` / CI: it lives beside the e2e suite but
 * has its own entry point (this file) and is never picked up by
 * `playwright test` (testDir there is `./tests`, this lives in `./shots`,
 * and this is a plain Node script, not a `*.spec.ts`). Run it manually:
 *
 *   node e2e/shots/capture.cjs [--fresh] [--only=1,2,3,...]
 *
 * Requires `cwebp` (Debian/Ubuntu: `apt install webp`; macOS:
 * `brew install webp`). Playwright captures a temporary PNG; this script
 * encodes it as lossless WebP and keeps only the WebP in site/img/.
 *
 * See docs/product/2026-07-01-web-screenshots.md for the shot list and the
 * conventions this follows (viewport, theme-per-shot, captions).
 *
 * Safety: this NEVER serves or writes into ~/alix-demo or ~/alix-kids
 * directly. Both are copied into e2e/shots/.tmp/ once (reused on later runs
 * unless --fresh) and only the copies are served/graded/augmented.
 */
const { chromium } = require("@playwright/test");
const { execFileSync, spawn } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");
const os = require("node:os");

const REPO_ROOT = path.join(__dirname, "..", "..");
const SHOTS_DIR = __dirname;
const WORK = path.join(SHOTS_DIR, ".tmp");
const DEMO_SRC = path.join(os.homedir(), "alix-demo");
const KIDS_SRC = path.join(os.homedir(), "alix-kids");
const DEMO_DIR = path.join(WORK, "demo");
const KIDS_DIR = path.join(WORK, "kids");
const KIDS_CONFIG = path.join(SHOTS_DIR, "kids.toml");
const OUT_DIR = path.join(REPO_ROOT, "site", "img");

const DEMO_PORT = 7801;
const KIDS_PORT = 7802;
const DEMO_BASE = `http://127.0.0.1:${DEMO_PORT}`;
const KIDS_BASE = `http://127.0.0.1:${KIDS_PORT}`;

// The one demo source (spec: "one demo deck for every shot"): the rust-book
// workspace. The hero fact deck (01) is the entry point of its `requires:`
// chain, pre-augmented (choices/notes/keypoints/topology) by ensureAugmented().
const HERO_DECK = "rust-book/01-the-stack-the-heap-and-the.md";
const HERO_FILE = path.join(DEMO_DIR, "rust-book", "01-the-stack-the-heap-and-the.md");
const TRACE_DECK = "rust-book/02-how-let-s2-s1-moves-a.md";
// runAugment("topology") passes no `--with`, so `alix deck augment` auto-names
// the generated topology "auto" (see AugmentTarget::Topology's default when
// no guidance is given) — this must match, or shot 8's topology-scoped
// /api/select silently finds no such topology.
const TOPOLOGY_NAME = "auto";

// User ruling 2026-07-11: one theme across every shot (the house default —
// see assets/web/theme.js's `DEFAULT`/THEMES[0], id "dark", name "alix") —
// not the spec's original per-shot variety. Shot 9 is the one place theme
// variety still shows (the popover's own swatch grid), so it's the only shot
// that touches the theme mechanism beyond this default.
const DEFAULT_THEME = "dark";

const VIEWPORT = { width: 1440, height: 900 };
const SCALE = 2;

const argv = process.argv.slice(2);
const FRESH = argv.includes("--fresh");
const onlyArg = argv.find((a) => a.startsWith("--only="));
const ONLY = onlyArg ? new Set(onlyArg.split("=")[1].split(",").map(Number)) : null;
const wants = (n) => !ONLY || ONLY.has(n);

function log(...a) {
  console.log("[shots]", ...a);
}
function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

function requireWebpEncoder() {
  try {
    execFileSync("cwebp", ["-version"], { stdio: "ignore" });
  } catch {
    throw new Error(
      "cwebp is required to capture site screenshots (Debian/Ubuntu: apt install webp; macOS: brew install webp)",
    );
  }
}

// ---- fixtures: copy once, never touch the originals -----------------------

function copyOnce(src, dest, label) {
  if (fs.existsSync(dest) && !FRESH) {
    log(`reusing existing ${label} copy at`, dest);
    return;
  }
  if (!fs.existsSync(src)) {
    throw new Error(`${label} source not found: ${src}`);
  }
  fs.rmSync(dest, { recursive: true, force: true });
  fs.mkdirSync(path.dirname(dest), { recursive: true });
  fs.cpSync(src, dest, { recursive: true });
  log(`copied ${label}:`, src, "->", dest);
}

// A snapshot of every progress/recent file's mtime+size under a decks root —
// compared before/after against the REAL ~/alix-demo and ~/alix-kids to prove
// this script never wrote into them.
function snapshotStoreFiles(root) {
  const out = {};
  const walk = (dir) => {
    if (!fs.existsSync(dir)) return;
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(p);
      else if (/^(progress|recent)\.json/.test(entry.name)) {
        const st = fs.statSync(p);
        out[p] = `${st.mtimeMs}:${st.size}`;
      }
    }
  };
  walk(root);
  return out;
}

function diffSnapshots(before, after) {
  const changed = [];
  for (const k of new Set([...Object.keys(before), ...Object.keys(after)])) {
    if (before[k] !== after[k]) changed.push(k);
  }
  return changed;
}

// ---- deck parsing (CommonMark card format) --------------------------------

function parseDeck(file) {
  const lines = fs.readFileSync(file, "utf8").split("\n");
  const cards = [];
  let cur = null;
  for (const raw of lines) {
    const line = raw.trim();
    if (line.startsWith("## ")) {
      cur = { front: line.slice(3).trim(), back: [] };
      cards.push(cur);
    } else if (!line || line.startsWith(">") || line.startsWith("<!--") || line.startsWith("---")) {
      // machine directive / note / frontmatter / blank — skip
    } else if (cur) {
      cur.back.push(line);
    }
  }
  return cards;
}

// ---- augment cache (choices/notes/keypoints/topology on the hero deck) ----

function heroAugmentState() {
  const augPath = path.join(DEMO_DIR, "rust-book", "augment.json");
  if (!fs.existsSync(augPath)) return { distractors: 0, note: 0, keypoints: 0, topology: false };
  const data = JSON.parse(fs.readFileSync(augPath, "utf8"));
  const cards = parseDeck(HERO_FILE);
  const heroIds = new Set(); // we don't have ids without the app; approximate by counting any card entries with each key
  let distractors = 0,
    note = 0,
    keypoints = 0;
  for (const v of Object.values(data.cards || {})) {
    if (v.distractors) distractors++;
    if (v.note) note++;
    if (v.keypoints) keypoints++;
  }
  const topology = Array.isArray(data.topologies) && data.topologies.length > 0;
  return { distractors, note, keypoints, topology, cardCount: cards.length };
}

function runAugment(target) {
  log("augmenting hero deck --target", target, "(real Claude call, this can take a while)…");
  execFileSync("alix", ["deck", "augment", HERO_FILE, "--target", target], {
    stdio: "inherit",
    cwd: REPO_ROOT,
  });
}

// `source: assets` (a bare directory, the "frozen snapshot" convention —
// see src/deck.rs Deck::is_frozen) makes exam generation fail outright:
// exam.rs's source_section() reads each `source:` value as a single file
// path and only supports " + "-joined multi-file lists, so a directory just
// errors ("Is a directory") and, with no readable source left, the exam
// bails with "none of the deck's `source:` paths could be read to examine
// against" — confirmed via a direct /api/exam/start probe. That looks like a
// real product gap (source_section doesn't expand a directory the way the
// citation/trace resolvers do), not something to patch in the app from a
// capture script — flagged in the report instead. Worked around HERE, in the
// scratch copy only, by pointing the frontmatter at the exact per-card files
// its own `at:` citations already name (01.md..10.md), which source_paths()
// already supports via " + ". Never touches ~/alix-demo.
function fixHeroSourceForExam() {
  const text = fs.readFileSync(HERO_FILE, "utf8");
  const files = Array.from(
    { length: 10 },
    (_, i) => `  - "assets/${String(i + 1).padStart(2, "0")}.md"`,
  ).join("\n");
  const fixed = text.replace(/^source:\s*["']?assets["']?\s*$/m, `source:\n${files}`);
  if (fixed !== text) {
    fs.writeFileSync(HERO_FILE, fixed);
    log("worked around the `source: assets` exam bug in the scratch copy (see comment above)");
  }
}

function ensureAugmented() {
  const state = heroAugmentState();
  log("hero deck augment cache:", state);
  // The hero deck has 10 cards; a handful (atomic answers) are skipped for
  // keypoints on purpose (augment.rs), so we only require "some" coverage,
  // not full coverage, before treating each target as already present.
  if (state.distractors < 5) runAugment("choices");
  if (state.note < 5) runAugment("notes");
  if (state.keypoints < 5) runAugment("keypoints");
  if (!state.topology) runAugment("topology");
}

// ---- server lifecycle -------------------------------------------------

const children = [];

// A crashed prior run (this script, or an ad-hoc debug session) can leave an
// `alix` process bound to DEMO_PORT/KIDS_PORT. Without this, startServer()'s
// child fails fast ("Address already in use") but waitForServer() still
// happily finds *something* answering /api/version on that port — the
// leftover process — and the run silently proceeds against stale state
// instead of this run's fresh copy. Best-effort; fine to no-op if `fuser`
// isn't installed or nothing's listening.
function freePort(port) {
  try {
    execFileSync("fuser", ["-k", `${port}/tcp`], { stdio: "ignore" });
  } catch {
    // nothing was listening, or `fuser` isn't installed — either way, fine
  }
}

function waitForServer(base, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs;
  return (async () => {
    while (Date.now() < deadline) {
      try {
        const res = await fetch(`${base}/api/version`);
        if (res.ok) return;
      } catch {
        // not up yet
      }
      await sleep(300);
    }
    throw new Error(`server at ${base} never came up`);
  })();
}

function startServer(dir, port, extraArgs = []) {
  const args = [dir, "--port", String(port), "--new", "20", ...extraArgs];
  log("starting: alix", args.join(" "));
  const child = spawn("alix", args, { cwd: REPO_ROOT, stdio: ["ignore", "pipe", "pipe"] });
  child.stdout.on("data", (d) => process.stdout.write(`[alix:${port}] ${d}`));
  child.stderr.on("data", (d) => process.stderr.write(`[alix:${port}] ${d}`));
  children.push(child);
  return child;
}

function stopAll() {
  for (const c of children) {
    try {
      c.kill("SIGTERM");
    } catch {
      // already gone
    }
  }
}
process.on("exit", stopAll);
for (const sig of ["SIGINT", "SIGTERM"]) {
  process.on(sig, () => {
    stopAll();
    process.exit(1);
  });
}

// ---- tiny JSON API client (talks straight to the running alix server) -----

async function api(base, method, urlPath, body) {
  const res = await fetch(`${base}${urlPath}`, {
    method,
    headers: body !== undefined ? { "content-type": "application/json" } : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) {
    throw new Error(`${method} ${urlPath} -> ${res.status}`);
  }
  const text = await res.text();
  return text ? JSON.parse(text) : null;
}

// ---- theme -----------------------------------------------------------

async function setTheme(page, base, id) {
  await page.goto(`${base}/`, { waitUntil: "domcontentloaded" });
  await page.evaluate((tid) => localStorage.setItem("alix-theme", tid), id);
  await page.reload({ waitUntil: "domcontentloaded" });
  await page.waitForTimeout(350);
}

// The header <alix-logo> (assets/web/alix-logo.js) is a custom element that
// plays a ~2.62s one-shot "birth" animation on every connect/reset (page
// load, theme reload, and — see review.html's updateBusy()/replayLogo() — it
// also replays after any AI call finishes, since the ask/exam poll loops
// toggle its `loop` attribute on and off). There's no class/attribute it
// flips when settled, but the component tracks its own progress internally
// (`_elapsed` vs. `restAt`, both plain instance properties — see
// alix-logo.js `_play()`/`get restAt()`), so poll those directly rather than
// guessing a fixed delay. Absent element (e.g. kids.html only creates one
// transiently, for its pre-load splash) resolves immediately — nothing to
// wait for.
async function settleLogo(page) {
  await page
    .waitForFunction(
      () => {
        const el = document.querySelector("alix-logo");
        if (!el) return true;
        if (typeof el._elapsed !== "number" || typeof el.restAt !== "number") return true;
        return el._elapsed >= el.restAt;
      },
      { timeout: 6000 },
    )
    .catch(() => log("WARNING: alix-logo did not settle within 6s — capturing anyway"));
}

// Wait for every currently-running CSS animation/transition on the page to
// finish (drawer/reveal/panel entrances: cardIn/deal/revealIn/DRAWER_MS, all
// ~0.2-0.35s per review.html's own comments) — covers anything a fixed sleep
// would guess at. Polled rather than a one-shot check: a finishing animation
// can itself trigger another (e.g. a re-render), so this settles only once
// nothing is running for one whole poll tick.
async function settleAnimations(page) {
  await page
    .waitForFunction(() => document.getAnimations().every((a) => a.playState !== "running"), { timeout: 3000 })
    .catch(() => log("WARNING: a CSS animation was still running after 3s — capturing anyway"));
}

// Every screenshot goes through here — settle the header logo's own
// requestAnimationFrame-driven "birth" animation (not a CSS animation, so
// settleAnimations() can't see it) and any CSS entrance transition, then a
// small buffer for the final paint, before the pixels are read.
// `ready` is a CSS selector unique to the screen this shot claims to show —
// required, not optional. Its own bug class (shot 4: the exam genuinely ran
// and finished server-side, but the screenshot was taken after a page
// *reload*, which only round-trips StateDto — exam/walk progress is
// client-side-only JS state, so the reload silently rendered the picker
// underneath instead) is exactly why this is enforced here, in one place,
// rather than left to each shot function to remember. A screen mismatch
// throws — never writes a WebP of the wrong screen; the caller's try/catch
// turns that into an honest SKIP in the summary.
async function shot(page, filename, ready) {
  if (path.extname(filename) !== ".webp") {
    throw new Error(`screenshot output must be .webp: ${filename}`);
  }
  await page
    .locator(ready)
    .first()
    .waitFor({ state: "visible", timeout: 10_000 });
  await settleLogo(page);
  await settleAnimations(page);
  await page.waitForTimeout(200);
  const out = path.join(OUT_DIR, filename);
  const stem = path.basename(filename, ".webp");
  const png = path.join(WORK, `${stem}.png`);
  const webp = path.join(WORK, filename);
  try {
    await page.screenshot({ path: png, type: "png" });
    execFileSync("cwebp", ["-quiet", "-lossless", "-z", "9", png, "-o", webp]);
    fs.renameSync(webp, out);
  } finally {
    fs.rmSync(png, { force: true });
    fs.rmSync(webp, { force: true });
  }
  log("wrote", path.relative(REPO_ROOT, out));
}

// ---- setup: establish real Recall schedules on the hero deck's 10 cards ---
// This single batch feeds THREE shots: (1) explain/keypoints needs a card
// established at Recall so Reconstruct is immediately due (skips the 60s
// acquire cooldown — see src/scheduler.rs `Fsrs::due_at`'s cross-depth
// immediacy rule); (8) the topology heatmap reads Recall retrievability.
async function establishHeroSchedules(page) {
  log("acquiring all hero-deck cards (phase 1/2)…");
  let s = await api(DEMO_BASE, "POST", "/api/select", { deck: HERO_DECK, depth: "recall", max_new: 20 });
  let guard = 0;
  while (s && s.kind === "review" && s.phase === "review" && guard++ < 20) {
    if (s.acquire) {
      s = await api(DEMO_BASE, "POST", "/api/acquire", {});
    } else {
      break;
    }
  }
  log("waiting out the acquire cooldown (~65s, real wall-clock, no AI call)…");
  await sleep(65_000);

  log("grading all hero-deck cards at Recall (phase 2/2)…");
  s = await api(DEMO_BASE, "POST", "/api/select", { deck: HERO_DECK, depth: "recall", max_new: 20 });
  guard = 0;
  const gradedIds = [];
  let idx = 0;
  while (s && s.kind === "review" && s.phase === "review" && guard++ < 20) {
    const front = s.card && s.card.front;
    // A representative red-to-green spread for the topology heatmap: mostly
    // pass, with a couple of misses/partials sprinkled in.
    const pattern = ["passed", "passed", "failed", "passed", "partly", "passed", "failed", "passed", "passed", "partly"];
    const grade = pattern[idx % pattern.length];
    idx++;
    gradedIds.push({ id: s.card && s.card.id, front, grade });
    s = await api(DEMO_BASE, "POST", "/api/grade", { grade });
  }
  log(
    "graded",
    gradedIds.length,
    "cards:",
    gradedIds.map((c) => `${c.front} -> ${c.grade}`).join("; "),
  );

  // Stagger review recency so the topology heatmap shows a genuine red->green
  // spread rather than "everything reviewed a second ago" (FSRS retrievability
  // is ~1.0 right at last_review_ms regardless of grade). This edits ONLY the
  // scratch copy's progress.json — never the real ~/alix-demo — and only the
  // `last_review_ms`/`due_ms` timestamps, not the grades or history just
  // recorded for real above. CardDto has no `id` field on the wire (by
  // design — see docs/API.md), so this reads the store file directly rather
  // than trying to correlate API responses to card ids.
  backdateRecallReviews();

  return gradedIds;
}

function backdateRecallReviews() {
  const storePath = path.join(DEMO_DIR, "rust-book", "progress.json");
  if (!fs.existsSync(storePath)) {
    log("WARNING: no progress.json to backdate at", storePath);
    return;
  }
  const store = JSON.parse(fs.readFileSync(storePath, "utf8"));
  const cards = store.cards || {};
  const ids = Object.keys(cards).filter((id) => cards[id] && cards[id].recall);
  const dayMs = 86_400_000;
  const spread = [0, 1, 2, 4, 6, 9, 12, 16, 20, 25]; // days back, one per card
  ids.forEach((id, i) => {
    const back = spread[i % spread.length] * dayMs;
    cards[id].recall.last_review_ms = Math.max(0, cards[id].recall.last_review_ms - back);
  });
  fs.writeFileSync(storePath, JSON.stringify(store, null, 2));
  log("backdated review recency on", ids.length, "cards for heatmap variety");
}

// ---- shot 1: explain-mode keypoints checklist -----------------------------

async function shot1(page) {
  log("== shot 1: explain-mode keypoints ==");
  await setTheme(page, DEMO_BASE, DEFAULT_THEME);
  let s = await api(DEMO_BASE, "POST", "/api/select", { deck: HERO_DECK, depth: "reconstruct", max_new: 20 });
  let guard = 0;
  // Skip cards without a cached keypoints list (a couple of atomic answers
  // were deliberately skipped by `alix deck augment --target keypoints`).
  while (s && s.kind === "review" && s.phase === "review" && guard++ < 15) {
    const hasKp = Array.isArray(s.keypoints) && s.keypoints.length > 0;
    if (hasKp) break;
    if (s.acquire) s = await api(DEMO_BASE, "POST", "/api/acquire", {});
    else s = await api(DEMO_BASE, "POST", "/api/grade", { grade: "passed" });
  }
  if (!s || !Array.isArray(s.keypoints) || s.keypoints.length === 0) {
    log("SKIP shot 1: no reconstruct-depth card with cached keypoints was reachable");
    return false;
  }
  log("shot 1 card:", s.card.front, "-", s.keypoints.length, "keypoints");
  await page.goto(`${DEMO_BASE}/`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(400);
  const answer = (s.card.back || []).join(" ");
  await page.locator(".explain-input").fill(answer);
  await page.locator(".explain-input").press("Shift+Enter");
  await page.waitForTimeout(400);
  // Mark every point via the legend's "Yes" chip (`answerKeypoint`), not by
  // clicking the `.kp-list li.pt` items directly: on a cited card (this deck's
  // cards all carry `at:` directives) the answer region ALSO has its own onclick
  // (source<->answer swap, review.html's `onCiteClick`), and a keypoint <li>
  // click bubbles into it — the first click silently swaps the whole panel to
  // the citation excerpt instead of marking the point. The legend chip lives
  // outside that region, so it doesn't bubble into it.
  const n = await page.locator(".kp-list li.pt").count();
  const yesBtn = page.getByRole("button", { name: "Yes" });
  for (let i = 0; i < n; i++) {
    await yesBtn.click();
    await page.waitForTimeout(120);
  }
  await page.waitForTimeout(300);
  await shot(page, "shot-1-verify.webp", ".kp-list");
  return true;
}

// ---- shot 2: ask-tutor panel (real Claude call) ---------------------------

async function shot2(page) {
  log("== shot 2: ask-tutor ==");
  await setTheme(page, DEMO_BASE, DEFAULT_THEME);
  // cram:true: on a re-run against a reused scratch copy, every Recall card
  // may already be graded and not due again for days — without it, /api/select
  // can come back with nothing current to review (a disabled/absent primary
  // chip, per shot 2's own earlier failure).
  let s = await api(DEMO_BASE, "POST", "/api/select", { deck: HERO_DECK, depth: "recall", max_new: 20, cram: true });
  let guard = 0;
  while (s && s.kind === "review" && s.phase === "review" && s.acquire && guard++ < 15) {
    s = await api(DEMO_BASE, "POST", "/api/acquire", {});
  }
  await page.goto(`${DEMO_BASE}/`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(400);
  // Reveal the card first — the "Ask tutor" chip only shows once answered.
  const revealBtn = page.locator(".chip.primary");
  if (await revealBtn.count()) await revealBtn.first().click();
  await page.waitForTimeout(300);
  const askChip = page.locator(".chip.ask");
  if (!(await askChip.count())) {
    log("SKIP shot 2: no .chip.ask visible after reveal");
    return false;
  }
  await askChip.first().click();
  await page.waitForTimeout(300);
  // The "Send" chip lives in the shared #legend footer, not inside
  // .ask-panel (renderAsk() appends it via legend.appendChild, a sibling of
  // the panel) — Shift+Enter on the textarea (the same keydown handler) is
  // simpler and matches how a real user sends it anyway.
  await page.locator(".ask-input").fill("Why is pushing to the stack faster than allocating on the heap?");
  await page.locator(".ask-input").press("Shift+Enter");
  log("waiting for the real tutor response (up to 120s)…");
  // Wait for an actual answer, not just ".ask-thinking" hidden: that div is
  // rebuilt on every ~400ms poll tick (fillAskLog), so it can be transiently
  // absent from the DOM between polls even while still thinking — a shot
  // landing in that gap captured a bare "Thinking…" panel with no answer.
  await page
    .locator(".ask-a")
    .first()
    .waitFor({ state: "visible", timeout: 120_000 })
    .catch(() => log("WARNING: no .ask-a appeared within 120s — capturing current state anyway"));
  await page.waitForTimeout(400);
  await shot(page, "shot-2-tutor.webp", ".ask-a");
  return true;
}

// ---- shot 3: multiple-choice with real AI distractors ---------------------

async function shot3(page) {
  log("== shot 3: multiple-choice ==");
  await setTheme(page, DEMO_BASE, DEFAULT_THEME);
  // Recognize is unscheduled/boolean — no acquire cooldown, so this is
  // reachable immediately, even on a totally fresh card. cram:true covers a
  // re-run where every card is already past its first Recognize pass.
  let s = await api(DEMO_BASE, "POST", "/api/select", {
    deck: HERO_DECK,
    depth: "recognize",
    max_new: 20,
    cram: true,
  });
  let guard = 0;
  while (s && s.kind === "review" && s.phase === "review" && guard++ < 15) {
    const hasChoices = Array.isArray(s.choices) && s.choices.length > 1;
    if (hasChoices) break;
    s = await api(DEMO_BASE, "POST", "/api/skip", {});
  }
  if (!s || !Array.isArray(s.choices) || s.choices.length < 2) {
    log("SKIP shot 3: no Recognize card with multiple choices was reachable");
    return false;
  }
  await page.goto(`${DEMO_BASE}/`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(400);
  await shot(page, "shot-3-modes.webp", ".options .option");
  return true;
}

// ---- shot 4: AI exam (real Claude generate + grade calls) -----------------

function bestAnswer(corpus, question) {
  const norm = (t) =>
    t
      .toLowerCase()
      .replace(/[^a-z0-9 ]/g, " ")
      .split(/\s+/)
      .filter((w) => w.length > 2);
  const qWords = new Set(norm(question));
  let best = null,
    bestScore = -1;
  for (const c of corpus) {
    const words = norm(c.front + " " + c.back.join(" "));
    const score = words.filter((w) => qWords.has(w)).length;
    if (score > bestScore) {
      bestScore = score;
      best = c;
    }
  }
  return best ? best.back.join(" ") : "";
}

// Exam progress is CLIENT-side JS state in review.html (`examData`, a plain
// top-level `let`) with NO server-side reload-resume — unlike a review
// session's StateDto, `GET /api/state` doesn't carry it. A first version of
// this shot drove the exam via raw node-side fetch + a final `page.goto` to
// render the result, and it silently screenshotted the PICKER: the server
// had genuinely finished (confirmed separately, by polling /api/exam
// directly), but the reload never told the *browser* about it. So this
// drives the exam through the page's own functions instead — reachable
// directly since they're plain identifiers in review.html's classic
// (non-module) <script>, exactly like `state`/`examData` are for reading.
function pollExam(page) {
  return page.evaluate(() => {
    if (typeof examData === "undefined" || !examData) return null;
    const { phase, thinking, current, total, question, on_last, error, passed } = examData;
    return { phase, thinking, current, total, question, on_last, error, passed };
  });
}

async function shot4(page) {
  log("== shot 4: AI exam ==");
  await setTheme(page, DEMO_BASE, DEFAULT_THEME);
  const corpus = parseDeck(HERO_FILE);
  await api(DEMO_BASE, "POST", "/api/deselect", {}).catch(() => {});
  await page.goto(`${DEMO_BASE}/`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(400);
  await page.evaluate((deck) => {
    // eslint-disable-next-line no-undef
    startExam(deck);
  }, HERO_DECK);

  // Generous timeouts (per the brief): generating 5 questions from a 10-file
  // concatenated source is genuinely slow — a first attempt at this shot gave
  // up at 60s while the server was still legitimately "generating" (confirmed
  // by polling past that point directly against the running server), so this
  // errs long rather than false-negative.
  const GENERATING_TIMEOUT_S = 240;
  const THINKING_TIMEOUT_S = 240;
  let e = await pollExam(page);
  let guard = 0;
  while ((!e || e.thinking) && guard++ < GENERATING_TIMEOUT_S) {
    await sleep(1000);
    e = await pollExam(page);
  }
  if (!e || e.phase === "cooldown" || e.error) {
    log("SKIP shot 4: exam unavailable (phase=", e && e.phase, "error=", e && e.error, ")");
    return false;
  }
  guard = 0;
  while (e && e.phase === "answering" && guard++ < 20) {
    const answer = bestAnswer(corpus, e.question || "");
    log(`exam Q${e.current}/${e.total}: ${e.question}`);
    log(`  -> answering with: ${answer.slice(0, 100)}`);
    if (e.on_last) {
      await page.evaluate((text) => {
        const ta = document.querySelector(".exam-input");
        if (ta) ta.value = text;
        // eslint-disable-next-line no-undef
        examSubmit();
      }, answer);
    } else {
      await page.evaluate(
        ({ text, dest }) => {
          const ta = document.querySelector(".exam-input");
          if (ta) ta.value = text;
          // eslint-disable-next-line no-undef
          examNav(dest);
        },
        { text: answer, dest: e.current + 1 },
      );
    }
    let waitGuard = 0;
    e = await pollExam(page);
    while (e && e.thinking && waitGuard++ < THINKING_TIMEOUT_S) {
      await sleep(1000);
      e = await pollExam(page);
    }
  }
  if (!e || e.phase !== "results") {
    log("SKIP shot 4: exam did not reach results (phase=", e && e.phase, ")");
    return false;
  }
  log("exam verdict: passed =", e.passed);
  await shot(page, "shot-4-exam.webp", ".exam-pass, .exam-fail");
  return true;
}

// ---- shot 5: augment screen -----------------------------------------------

async function shot5(page) {
  log("== shot 5: augment screen ==");
  await setTheme(page, DEMO_BASE, DEFAULT_THEME);
  await api(DEMO_BASE, "POST", "/api/deselect", {}).catch(() => {});
  await page.goto(`${DEMO_BASE}/`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(400);
  const workspaceRow = page.locator(".deckrow").filter({ hasText: "Rust" }).first();
  await workspaceRow.click();
  await page.waitForTimeout(400);
  const heroRow = page.locator(".deckrow").filter({ hasText: "Stack" }).first();
  await heroRow.click();
  await page.waitForTimeout(250);
  const augBtn = page.getByRole("button", { name: "Augment" });
  if (!(await augBtn.count())) {
    log("SKIP shot 5: no Augment chip visible for the focused deck");
    return false;
  }
  await augBtn.first().click();
  await page.waitForTimeout(400);
  await shot(page, "shot-5-augment.webp", ".aug-card");
  return true;
}

// ---- shot 6: trace walk checkpoint -----------------------------------------

async function shot6(page) {
  log("== shot 6: trace walk ==");
  await setTheme(page, DEMO_BASE, DEFAULT_THEME);
  await api(DEMO_BASE, "POST", "/api/deselect", {}).catch(() => {});
  // A walk isn't resumable across a hard reload the way a review session is
  // (GET /api/state only round-trips StateDto) — launch it through the real
  // picker click flow instead, same as a user would.
  await page.goto(`${DEMO_BASE}/`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(400);
  const workspaceRow = page.locator(".deckrow").filter({ hasText: "THE Rust book" }).first();
  await workspaceRow.click();
  await page.waitForTimeout(400);
  const traceRow = page.locator(".deckrow").filter({ hasText: "Moves a String" }).first();
  if (!(await traceRow.count())) {
    log("SKIP shot 6: trace deck row not found in the drilled workspace");
    return false;
  }
  await traceRow.click();
  await page.waitForTimeout(250);
  const learnBtn = page.getByRole("button", { name: /^Learn/ });
  if (await learnBtn.count()) await learnBtn.first().click();
  else await traceRow.press("Enter");
  await page.waitForTimeout(500);
  const field = page.locator(".wfield");
  if (await field.count()) {
    await field.fill("It moves — s1 is invalidated and s2 becomes the sole owner of the heap data.");
    await field.press("Shift+Enter");
    await page.waitForTimeout(400);
  }
  if (!(await page.locator(".wexcerpt").count())) {
    log("SKIP shot 6: no .wexcerpt rendered — walk did not reach the reveal phase");
    return false;
  }
  await shot(page, "shot-6-trace.webp", ".wexcerpt");
  return true;
}

// ---- shot 7: picker, workspace expanded ------------------------------------

async function shot7(page) {
  log("== shot 7: picker ==");
  await setTheme(page, DEMO_BASE, DEFAULT_THEME);
  await api(DEMO_BASE, "POST", "/api/deselect", {}).catch(() => {});
  await page.goto(`${DEMO_BASE}/`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(400);
  const workspaceRow = page.locator(".deckrow").filter({ hasText: "Rust" }).first();
  await workspaceRow.click();
  await page.waitForTimeout(400);
  // .tree: the dependency-tree branch-line guides only render once drilled
  // INTO the workspace — confirms this isn't still the collapsed list.
  await shot(page, "shot-7-picker.webp", ".deckrow .tree");
  return true;
}

// ---- shot 8: topology heatmap in review ------------------------------------

async function shot8(page) {
  log("== shot 8: topology heatmap ==");
  await setTheme(page, DEMO_BASE, DEFAULT_THEME);
  await api(DEMO_BASE, "POST", "/api/deselect", {}).catch(() => {});
  const s = await api(DEMO_BASE, "POST", "/api/select", {
    deck: HERO_DECK,
    topology: TOPOLOGY_NAME,
    depth: "recall",
    max_new: 20,
  });
  if (!s || s.kind !== "review") {
    log("SKIP shot 8: topology-scoped select did not return a review session");
    return false;
  }
  await page.goto(`${DEMO_BASE}/`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(400);
  const crumb = page.locator("#crumbStrip");
  if (!(await crumb.count())) {
    log("SKIP shot 8: no #crumbStrip rendered for this session");
    return false;
  }
  await shot(page, "shot-8-topology.webp", "#crumbStrip");
  return true;
}

// ---- shot 9: theme gallery --------------------------------------------------

async function shot9(page) {
  log("== shot 9: theme gallery ==");
  // User ruling: every OTHER shot stays on DEFAULT_THEME — this is the one
  // place theme variety still shows, via the popover's own swatch grid, not
  // by committing a different theme to the app. Leave the default active.
  await setTheme(page, DEMO_BASE, DEFAULT_THEME);
  // #theme-open lives inside the ☰ menu (role="menuitem") — open that first.
  await page.locator("#kebab").click();
  await page.waitForTimeout(200);
  await page.locator("#theme-open").click();
  await page.waitForTimeout(250);
  await shot(page, "shot-9-themes.webp", ".theme-panel.show");
  return true;
}

// ---- shot 10: kids client ---------------------------------------------------

async function shot10(page) {
  log("== shot 10: kids client ==");
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto(`${KIDS_BASE}/`, { waitUntil: "domcontentloaded" });
  await page.waitForTimeout(500);
  const box = page.locator(".box").first();
  if (!(await box.count())) {
    log("SKIP shot 10: no .box on the kids home screen");
    return false;
  }
  await box.click();
  await page.waitForTimeout(400);
  // Not just .first(): the Animals box's alphabetically-first deck
  // (life-cycles.md) is an ORDERED-SEQUENCE answer (Egg/Caterpillar/
  // Chrysalis/Butterfly) that Recognize can't build real MC distractors for
  // (`choices` comes back null, an honest fallback — see isRecognizeFallback
  // in review.html), so it renders a reveal prompt, not tap options.
  // wild-animals.md has authored distractors on its first card.
  let deckRow = page.locator(".deck-row", { hasText: "wild-animals" }).first();
  if (!(await deckRow.count())) deckRow = page.locator(".deck-row").first();
  if (!(await deckRow.count())) {
    log("SKIP shot 10: no .deck-row inside the box");
    return false;
  }
  await deckRow.click();
  await page.waitForTimeout(300);
  const tapBtn = page.getByRole("button", { name: "Tap the answer" });
  if (await tapBtn.count()) {
    await tapBtn.click();
    await page.waitForTimeout(400);
  }
  const opts = page.locator(".opt-btn");
  if (!(await opts.count())) {
    log("SKIP shot 10: no .opt-btn tap-the-answer options rendered");
    return false;
  }
  // Pick the correct option so the "Got it!" + mascot state shows.
  const correctText = await page.evaluate(() => {
    try {
      // eslint-disable-next-line no-undef
      return state && state.card && state.card.back && state.card.back[0];
    } catch {
      return null;
    }
  });
  let target = opts.first();
  if (correctText) {
    const byText = page.locator(".opt-btn", { hasText: correctText });
    if (await byText.count()) target = byText.first();
  }
  await target.click();
  await page.waitForTimeout(500);
  await shot(page, "shot-10-kids.webp", ".opt-correct");
  return true;
}

// ---- main ------------------------------------------------------------------

async function main() {
  requireWebpEncoder();
  fs.mkdirSync(OUT_DIR, { recursive: true });
  fs.mkdirSync(WORK, { recursive: true });

  const beforeDemo = snapshotStoreFiles(DEMO_SRC);
  const beforeKids = snapshotStoreFiles(KIDS_SRC);

  copyOnce(DEMO_SRC, DEMO_DIR, "alix-demo");
  copyOnce(KIDS_SRC, KIDS_DIR, "alix-kids");

  fixHeroSourceForExam();
  if (wants(1) || wants(2) || wants(3) || wants(8)) ensureAugmented();

  freePort(DEMO_PORT);
  freePort(KIDS_PORT);
  startServer(DEMO_DIR, DEMO_PORT);
  startServer(KIDS_DIR, KIDS_PORT, ["--config", KIDS_CONFIG]);
  await waitForServer(DEMO_BASE);
  await waitForServer(KIDS_BASE);

  const browser = await chromium.launch();
  const context = await browser.newContext({ viewport: VIEWPORT, deviceScaleFactor: SCALE });
  const page = await context.newPage();

  const results = {};
  try {
    // Cheap setup that several shots depend on, done once up front regardless
    // of --only so re-running a single shot later stays fast.
    if (wants(1) || wants(8)) {
      await establishHeroSchedules(page);
    }

    const steps = [
      [1, shot1],
      [2, shot2],
      [3, shot3],
      [4, shot4],
      [5, shot5],
      [6, shot6],
      [7, shot7],
      [8, shot8],
      [9, shot9],
      [10, shot10],
    ];
    for (const [n, fn] of steps) {
      if (!wants(n)) continue;
      try {
        results[n] = await fn(page);
      } catch (err) {
        console.error(`[shots] shot ${n} FAILED:`, err.message);
        results[n] = false;
      }
    }
  } finally {
    await browser.close();
    stopAll();
  }

  await sleep(500);
  const afterDemo = snapshotStoreFiles(DEMO_SRC);
  const afterKids = snapshotStoreFiles(KIDS_SRC);
  const demoChanged = diffSnapshots(beforeDemo, afterDemo);
  const kidsChanged = diffSnapshots(beforeKids, afterKids);

  log("=== summary ===");
  for (const [n, ok] of Object.entries(results)) {
    log(`shot ${n}: ${ok ? "captured" : "SKIPPED"}`);
  }
  log("~/alix-demo files changed:", demoChanged.length ? demoChanged : "none");
  log("~/alix-kids files changed:", kidsChanged.length ? kidsChanged : "none");
  if (demoChanged.length || kidsChanged.length) {
    console.error("[shots] WARNING: real demo/kids progress files changed — investigate before trusting this run");
    process.exitCode = 1;
  }
}

main().catch((err) => {
  console.error(err);
  stopAll();
  process.exit(1);
});
