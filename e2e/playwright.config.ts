import path from "node:path";
import { defineConfig, devices } from "@playwright/test";

const ROOT = __dirname;
const REPO_ROOT = path.join(ROOT, "..");
const PREPARE_SCRIPT = path.join(ROOT, "prepare-fixtures.cjs");

// One `alix` server config per web client under test, each on its own port
// and its own scratch decks copy (see prepare-fixtures.cjs) — so the two
// servers' progress.json/recent.json writes never collide.
const KIDS_CONFIG = path.join(ROOT, "fixtures", "kids.toml");
const KIDS_DECKS_DIR = path.join(ROOT, ".tmp", "kids", "decks");
const KIDS_PORT = 7788;
const KIDS_BASE_URL = `http://127.0.0.1:${KIDS_PORT}`;

const ADULT_CONFIG = path.join(ROOT, "fixtures", "adult.toml");
const ADULT_DECKS_DIR = path.join(ROOT, ".tmp", "adult", "decks");
const ADULT_PORT = 7789;
const ADULT_BASE_URL = `http://127.0.0.1:${ADULT_PORT}`;

export default defineConfig({
  testDir: "./tests",
  globalSetup: require.resolve("./global-setup.ts"),
  // A bit above the 30s default: the webServer readiness probe only needs the
  // port to answer, but right after `cargo run` finishes a fresh build (e.g.
  // after editing an `include_str!`-embedded HTML asset), the very first real
  // page load can still be slow for a few seconds under that build's tail-end
  // CPU load. Everything after warms up and finishes in well under a second.
  timeout: 60_000,
  // Each server has one review session live on it at a time — tests read and
  // build on each other's server-side state, so they must not run
  // concurrently (within a project, or across the two).
  fullyParallel: false,
  workers: 1,
  reporter: "list",
  // Pinned explicitly (rather than Playwright's config-relative default) so
  // an ad-hoc `playwright test` invoked from the repo root can never scatter
  // trace/screenshot artifacts outside e2e/.
  outputDir: path.join(ROOT, "test-results"),
  use: {
    trace: "retain-on-failure",
  },
  webServer: [
    {
      // Prepares the scratch decks dir itself (not just via globalSetup —
      // see prepare-fixtures.cjs) so it's guaranteed to exist before
      // `cargo run` checks for it, then runs the real binary against it.
      // `--config` is required: without it `alix` would read the developer's
      // real platform config — their real decks dir and AI backend.
      command:
        `node "${PREPARE_SCRIPT}" kids && ` +
        `cargo run --quiet -- --config "${KIDS_CONFIG}" "${KIDS_DECKS_DIR}" --port ${KIDS_PORT}`,
      cwd: REPO_ROOT,
      url: KIDS_BASE_URL,
      timeout: 180_000, // cargo may need to build first
      // Never reuse: a server carries session state (and a handle on a decks
      // dir this config wipes and recopies each run). Reusing one that a
      // previous run left mid-session made `/` render the review screen
      // instead of the picker — a test that passed alone and failed in suite.
      // The binary is already built, so a fresh spawn costs ~a second.
      reuseExistingServer: false,
    },
    {
      command:
        `node "${PREPARE_SCRIPT}" adult && ` +
        `cargo run --quiet -- --config "${ADULT_CONFIG}" "${ADULT_DECKS_DIR}" --port ${ADULT_PORT}`,
      cwd: REPO_ROOT,
      url: ADULT_BASE_URL,
      timeout: 180_000,
      // Never reuse: a server carries session state (and a handle on a decks
      // dir this config wipes and recopies each run). Reusing one that a
      // previous run left mid-session made `/` render the review screen
      // instead of the picker — a test that passed alone and failed in suite.
      // The binary is already built, so a fresh spawn costs ~a second.
      reuseExistingServer: false,
    },
  ],
  projects: [
    {
      name: "kids",
      testMatch: /kids-.*\.spec\.ts/,
      use: { ...devices["Desktop Chrome"], baseURL: KIDS_BASE_URL },
    },
    {
      name: "adult",
      testMatch: /adult-.*\.spec\.ts/,
      use: { ...devices["Desktop Chrome"], baseURL: ADULT_BASE_URL },
    },
  ],
});
