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

// The run's whole exit decision, in one place so it can be pinned: a failed
// requested shot and a mutation of the real demo or kids stores are
// independent reasons to fail.
function exitCodeFor({ failed, demoChanged, kidsChanged }) {
  const any = (list) => Array.isArray(list) && list.length > 0;
  return any(failed) || any(demoChanged) || any(kidsChanged) ? 1 : 0;
}

module.exports = { runRequested, summarize, exitCodeFor };
