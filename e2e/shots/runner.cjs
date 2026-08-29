/**
 * The capture run's orchestration decision, kept out of capture.cjs so it can
 * be tested where `@playwright/test` is not installed (the `unit-js` job).
 */

// `--only` is the mechanism for not requesting a shot, so anything requested
// ends captured or failed: a `false` return and a thrown error mean the same
// thing, that the state the slide photographs was never reached.
//
// `captured` is the live receipt the capture writes to after each rename. The
// difference across one step is what THAT step wrote, so a shot writing
// another shot's filename cannot be credited to it, and a step writing two
// files fails until the protocol is changed on purpose.
async function runRequested(steps, wants, page, captured = null) {
  const results = {};
  for (const [n, fn] of steps) {
    if (!wants(n)) continue;
    const before = captured ? new Set(captured) : null;
    try {
      results[n] = await fn(page);
    } catch (err) {
      console.error(`[shots] shot ${n} FAILED:`, err.message);
      results[n] = false;
    }
    if (!results[n] || !captured) continue;
    const wrote = [...captured].filter((name) => !before.has(name));
    const mine = `shot-${n}-`;
    if (wrote.length !== 1 || !wrote[0].startsWith(mine)) {
      console.error(
        `[shots] shot ${n} reported captured but wrote ${JSON.stringify(wrote)}`,
      );
      results[n] = false;
    }
  }
  return results;
}

// `--only` takes shot numbers, and a number naming no step means the run did
// nothing while every later gate stayed green, so an unknown request is a
// failure rather than an empty selection.
function unknownRequests(steps, only) {
  if (!only) return [];
  const known = new Set(steps.map(([n]) => n));
  return [...only].filter((n) => !known.has(n));
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
// requested shot, an unknown `--only` number, and a mutation of the real demo
// or kids stores are independent reasons to fail.
function exitCodeFor({ failed, demoChanged, kidsChanged, unknown }) {
  const any = (list) => Array.isArray(list) && list.length > 0;
  return any(failed) || any(demoChanged) || any(kidsChanged) || any(unknown)
    ? 1
    : 0;
}

module.exports = { runRequested, summarize, unknownRequests, exitCodeFor };
