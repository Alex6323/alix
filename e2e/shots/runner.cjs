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

// A shot is proven captured only by a file THIS run wrote, so a WebP left on
// disk by an earlier run cannot stand in for one that was never taken. Each
// shotN writes exactly one `shot-N-*.webp`, so the receipt is matched by
// leading number rather than by a filename list that would go stale against
// the capture.
function unreceipted(shots, capturedFilenames) {
  const wrote = new Set();
  for (const name of capturedFilenames) {
    const match = /^shot-(\d+)-.*\.webp$/.exec(name);
    if (match) wrote.add(Number(match[1]));
  }
  return shots.filter((n) => !wrote.has(n));
}

// The run's whole exit decision, in one place so it can be pinned: a failed
// requested shot, a shot that reported success without writing its file, and a
// mutation of the real demo or kids stores are independent reasons to fail.
function exitCodeFor({ failed, demoChanged, kidsChanged, unwritten }) {
  const any = (list) => Array.isArray(list) && list.length > 0;
  return any(failed) || any(demoChanged) || any(kidsChanged) || any(unwritten)
    ? 1
    : 0;
}

module.exports = { runRequested, summarize, unreceipted, exitCodeFor };
