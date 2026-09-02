// The kids client on the zero-cooldown server (see playwright.config.ts,
// "kids-graded"): the one place a kid's card can be a real graded quiz in a
// single run. Everything else about the kids client lives in the ordinary
// kids project.
import { test, expect } from "./helpers";
import { kidsDeckRow, openApp } from "./helpers";

// The honest-grading rule (fix c46dad5): a wrong pick on a real Recognize
// quiz offers only "Keep going" (grades failed), never "Got it!". With no
// introduction cooldown, the card just acknowledged is due again at once and
// the same session serves it straight back as that quiz.
test("a wrong Recognize pick can only record failed, never passed", async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await openApp(page);
  await page.locator(".box", { hasText: "Animals" }).click();
  await kidsDeckRow(page, "wild").click();
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/select")),
    page.getByRole("button", { name: "Tap the answer" }).click(),
  ]);
  const prompt = page.locator(".rev-prompt");
  await expect(prompt).toBeVisible();
  const firstFront = await prompt.textContent();
  const answer = firstFront?.includes("tallest") ? "Giraffe" : "Cheetah";

  // First meeting: a pick acknowledges, and only "Got it! Next" follows.
  await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/choose")),
    page.locator(".opt-btn").filter({ hasText: answer }).click(),
  ]);
  await expect(page.locator(".rate-got")).toHaveCount(0);
  const [introduceResponse] = await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/introduce") && res.request().method() === "POST"),
    page.getByRole("button", { name: "Got it! Next" }).click(),
  ]);
  expect(introduceResponse.status()).toBe(200);
  const introduced = await introduceResponse.json();
  expect(introduced.introducing, "the acknowledged card returns as a real quiz").toBe(false);
  await expect(prompt).toHaveText(firstFront ?? "");

  const wrongOption = page.locator(".opt-btn").filter({ hasNotText: answer }).first();
  const [chooseResponse] = await Promise.all([
    page.waitForResponse((res) => res.url().includes("/api/choose") && res.request().method() === "POST"),
    wrongOption.click(),
  ]);
  expect(chooseResponse.status()).toBe(200);
  expect((await chooseResponse.json()).passed).toBe(false);

  await expect(page.locator(".opt-wrong")).toHaveCount(1);
  await expect(page.locator(".rate-got")).toHaveCount(0);
  await expect(page.locator(".rate-quiet")).toHaveCount(0);
  await expect(page.locator(".rate-again")).toBeVisible();

  const [gradeRequest, gradeResponse] = await Promise.all([
    page.waitForRequest((req) => req.url().includes("/api/grade") && req.method() === "POST"),
    page.waitForResponse((res) => res.url().includes("/api/grade") && res.request().method() === "POST"),
    page.locator(".rate-again").click(),
  ]);
  expect(gradeRequest.postDataJSON()).toEqual({ grade: "failed" });
  expect(gradeResponse.status()).toBe(200);
});
