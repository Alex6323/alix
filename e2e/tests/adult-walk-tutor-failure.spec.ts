// Deliberately uses the unextended Playwright `test`: the repro fails a request
// on purpose, and helpers.ts's auto fixture fails any test that logs a console
// error, which a deliberately-failed fetch always does. The assertions below
// are the subject of the test; the console noise is not.
import { test, expect } from "@playwright/test";
import { adultDeckRow, openApp } from "./helpers";

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await openApp(page);
});

// A failed tutor request inside a walk falls back to `study.load()`, which
// fetches `/api/state`. During a walk that endpoint answers with the review
// snapshot (`kind:"review"`, `phase:"select"`), never the walk, so applying it
// must not be read as evidence that the walk ended.
test("a failed tutor request during a walk does not eject the learner to the picker", async ({ page }) => {
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "Inline Trace").click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: "Learn" }).click(),
  ]);

  await page.locator(".wfield").fill("it grows first");
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/walk/predict")),
    page.getByRole("button", { name: "Reveal" }).click(),
  ]);
  await expect(page.locator(".wpoints .wpt code")).toHaveText("reserve");

  await page.getByRole("button", { name: "Ask tutor" }).click();
  await expect(page.locator(".ask-input")).toBeVisible();

  // The tutor question fails the way a restarted server or a dropped
  // connection makes it fail: the POST never answers.
  await page.route("**/api/walk/ask", (route) =>
    route.request().method() === "POST" ? route.abort() : route.continue(),
  );
  await page.locator(".ask-input").fill("why reserve first?");
  await page.locator(".ask-input").press("Shift+Enter");
  await page.waitForResponse((response) => response.url().includes("/api/state"));
  await page.waitForTimeout(400);

  // The trace the learner was walking is still the current subject, and the
  // deck picker has not taken over the screen.
  expect(await page.locator(".deckrow").count()).toBe(0);
  await expect(page.locator("#deck")).toHaveText("How push grows a Vec");
});
