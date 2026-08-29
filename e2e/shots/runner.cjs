/**
 * The capture run's orchestration decision, kept out of capture.cjs so it can
 * be tested where `@playwright/test` is not installed (the `unit-js` job).
 */

function registryProblems(shots) {
  if (!Array.isArray(shots) || shots.length === 0) {
    return ["the shot registry is not a nonempty array"];
  }
  const problems = [];
  const ids = new Set();
  for (const row of shots) {
    if (!Array.isArray(row) || row.length !== 3) {
      problems.push(`a row is not [number, file, producer]: ${JSON.stringify(row)}`);
      continue;
    }
    const [n, out, run] = row;
    if (!Number.isInteger(n) || n < 1) {
      problems.push(`a row's id is not a positive integer: ${JSON.stringify(n)}`);
      continue;
    }
    if (ids.has(n)) problems.push(`shot ${n} is registered twice`);
    ids.add(n);
    if (typeof out !== "string" || !new RegExp(`^shot-${n}-[a-z0-9]+\\.webp$`).test(out)) {
      problems.push(`shot ${n} must publish shot-${n}-<name>.webp, not ${JSON.stringify(out)}`);
    }
    if (typeof run !== "function" || run.name !== `shot${n}`) {
      problems.push(`shot ${n} must be produced by shot${n}, not ${run && run.name}`);
    }
  }
  return problems;
}

async function runRequested(shots, wants, page, captured = null) {
  const problems = registryProblems(shots);
  if (problems.length) {
    for (const problem of problems) {
      console.error(`[shots] ${problem}`);
    }
    throw new Error(`the shot registry is invalid: ${problems.join("; ")}`);
  }
  const results = {};
  for (const [n, out, run] of shots) {
    if (!wants(n)) continue;
    const before = captured ? new Set(captured) : null;
    try {
      results[n] = await run(page, out);
    } catch (err) {
      console.error(`[shots] shot ${n} FAILED:`, err.message);
      results[n] = false;
    }
    if (!results[n] || !captured) continue;
    const wrote = [...captured].filter((name) => !before.has(name));
    if (wrote.length !== 1 || wrote[0] !== out) {
      console.error(
        `[shots] shot ${n} reported captured but wrote ${JSON.stringify(wrote)}, not ${out}`,
      );
      results[n] = false;
    }
  }
  return results;
}

function unknownRequests(shots, only) {
  if (!only) return [];
  const known = new Set(shots.map(([n]) => n));
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

module.exports = {
  registryProblems,
  runRequested,
  summarize,
  unknownRequests,
  exitCodeFor,
};
