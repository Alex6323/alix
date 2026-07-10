import path from "node:path";
import { defineConfig, devices } from "@playwright/test";

const ROOT = __dirname;
const REPO_ROOT = path.join(ROOT, "..");
// One `alix` server config per client under test; an ADULT_CONFIG sibling
// (and its own webServer/project) lands when the adult app gets a spec here.
const KIDS_CONFIG = path.join(ROOT, "fixtures", "kids.toml");
const DECKS_DIR = path.join(ROOT, ".tmp", "decks");
const PREPARE_SCRIPT = path.join(ROOT, "prepare-fixtures.cjs");
const PORT = 7788;
const BASE_URL = `http://127.0.0.1:${PORT}`;

export default defineConfig({
  testDir: "./tests",
  globalSetup: require.resolve("./global-setup.ts"),
  // A bit above the 30s default: the webServer readiness probe only needs the
  // port to answer, but right after `cargo run` finishes a fresh build (e.g.
  // after editing an `include_str!`-embedded HTML asset), the very first real
  // page load can still be slow for a few seconds under that build's tail-end
  // CPU load. Everything after warms up and finishes in well under a second.
  timeout: 60_000,
  // One shared `alix` server, one review session live on it at a time — tests
  // read and build on each other's server-side state, so they must not run
  // concurrently.
  fullyParallel: false,
  workers: 1,
  reporter: "list",
  use: {
    baseURL: BASE_URL,
    trace: "retain-on-failure",
  },
  webServer: {
    // Prepares the scratch decks dir itself (not just via globalSetup — see
    // prepare-fixtures.cjs) so it's guaranteed to exist before `cargo run`
    // checks for it, then runs the real binary against it.
    command:
      `node "${PREPARE_SCRIPT}" && ` +
      `cargo run --quiet -- --config "${KIDS_CONFIG}" "${DECKS_DIR}" --port ${PORT}`,
    cwd: REPO_ROOT,
    url: BASE_URL,
    timeout: 180_000, // cargo may need to build first
    reuseExistingServer: !process.env.CI,
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
});
