// The kids tutor's answer arrives only through its setInterval poll: the
// send POST returns a thinking state, and the reply appears when a later
// polled GET stops thinking. This pins the polling path end to end,
// including the window-bound timer wiring, whose unbound form threw
// `Illegal invocation` in Chromium and shipped that way in v0.7.0 (the
// error-toast site of the same bug has its law in
// kids-image-mask-review.spec.ts; this is the tutor site).
import { test, expect } from "./helpers";
import { kidsDeckRow, openApp } from "./helpers";
import type { Page } from "@playwright/test";

function state() {
  return {
    kind: "review",
    study_revision: 1,
    phase: "review",
    card: {
      id: "kids-tutor-poll-card",
      front: "Why is the sky blue?",
      front_runs: [{ text: "Why is the sky blue?" }],
      front_units: null,
      context: [],
      context_runs: [],
      back: ["Rayleigh scattering"],
      back_runs: [[{ text: "Rayleigh scattering" }]],
      back_units: [{ kind: "sentence", text: "Rayleigh scattering" }],
      reshaped: false,
      note: [],
      images: [],
      images_back: [],
      citations: [],
      crumb: null,
    },
    choices: null,
    choice_runs: null,
    keypoints: null,
    keypoint_runs: null,
    introducing: false,
    mode: "flip",
    depth: "recall",
    input: "type",
    remaining: 1,
    initial: 1,
    reviews: 0,
    passed: 0,
    failed: 0,
    introduced: 0,
    exam_due: [],
    can_restart: false,
    promotable: false,
    next_due_ms: null,
    due_left: 0,
    new_left: 0,
    label: "kids-tutor-poll.md",
    save_error: null,
  };
}

async function openSyntheticCard(page: Page) {
  await page.route("**/api/select", (route) => route.fulfill({ json: state() }));
  await openApp(page);
  await page.locator(".box", { hasText: "Animals" }).click();
  await kidsDeckRow(page, "wild").click();
  await page.getByRole("button", { name: "Say it yourself" }).click();
}

test("a tutor answer arrives through the poll after the send returns thinking", async ({ page }) => {
  const idle = { transcript: [], thinking: false, status: null, error: null };
  const thinking = { transcript: [], thinking: true, status: null, error: null };
  const answered = {
    transcript: [{ q: "why?", a: "Because sunlight scatters." }],
    thinking: false,
    status: null,
    error: null,
  };

  let sent = false;
  let pollsAfterSend = 0;
  await page.route("**/api/ask", (route) => {
    if (route.request().method() === "POST") {
      sent = true;
      return route.fulfill({ json: thinking });
    }
    if (!sent) return route.fulfill({ json: idle });
    pollsAfterSend += 1;
    return route.fulfill({ json: pollsAfterSend < 2 ? thinking : answered });
  });

  await openSyntheticCard(page);
  await page.locator(".ask-btn", { hasText: "Ask Alix" }).click();
  await expect(page.locator("#askOverlay")).toBeVisible();

  await page.locator("#askInput").fill("why?");
  await page.locator("#askSendBtn").click();
  await expect(
    page.locator(".ask-bubble-think"),
    "the send response carries no answer, only the thinking state",
  ).toBeVisible();

  await expect(
    page.locator(".ask-bubble-a", { hasText: "Because sunlight scatters." }),
    "only a polled GET can deliver the reply, so this pins the interval wiring",
  ).toBeVisible();
  expect(pollsAfterSend, "the reply took more than one polled GET").toBeGreaterThanOrEqual(2);
});
