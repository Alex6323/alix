/**
 * The capture run's orchestration decision, kept out of capture.cjs so it can
 * be tested where `@playwright/test` is not installed (the `unit-js` job).
 */

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

function exitCodeFor({ failed, demoChanged, kidsChanged, unknown }) {
  const any = (list) => Array.isArray(list) && list.length > 0;
  return any(failed) || any(demoChanged) || any(kidsChanged) || any(unknown)
    ? 1
    : 0;
}

module.exports = { runRequested, summarize, unknownRequests, exitCodeFor };
