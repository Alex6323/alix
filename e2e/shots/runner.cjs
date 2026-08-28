/**
 * The capture run's orchestration decision, kept out of capture.cjs so it can
 * be tested where `@playwright/test` is not installed (the `unit-js` job).
 */

// `--only` is the mechanism for not requesting a shot, so anything requested
// ends captured or failed: a `false` return and a thrown error mean the same
// thing, that the state the slide photographs was never reached.
async function runRequested(steps, wants, page) {
  const results = {};
  for (const [n, fn] of steps) {
    if (!wants(n)) continue;
    try {
      results[n] = await fn(page);
    } catch (err) {
      console.error(`[shots] shot ${n} FAILED:`, err.message);
      results[n] = false;
    }
  }
  return results;
}

function summarize(results) {
  const lines = [];
  const failed = [];
  for (const [n, ok] of Object.entries(results)) {
    lines.push(`shot ${n}: ${ok ? "captured" : "FAILED"}`);
    if (!ok) failed.push(Number(n));
  }
  return { lines, failed };
}

module.exports = { runRequested, summarize };
