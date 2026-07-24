import { test, expect } from "./helpers";
import { adultDeckRow, openApp } from "./helpers";

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await openApp(page);
});

test("trace checkpoints render authored inline code", async ({ page }) => {
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "Inline Trace").click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: "Learn" }).click(),
  ]);

  await expect(page.locator("#deck code")).toHaveText(["push", "Vec"]);
  await expect(page.locator(".front-text code")).toHaveText("push");
  await expect(page.locator(".given code")).toHaveText("Vec");

  await page.locator(".wfield").fill("it grows first");
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/walk/predict")),
    page.getByRole("button", { name: "Reveal" }).click(),
  ]);

  await expect(page.locator(".wpoints .wpt code")).toHaveText("reserve");
  await expect(page.locator(".note code")).toHaveText("reserve");
  await expect(page.locator(".wpoints .wpt")).not.toContainText("`");
  await expect(page.locator(".source-excerpt .source-file")).toContainText("trace-source.txt");
  await expect(page.locator(".source-line")).toHaveCount(1);
  await expect(page.locator(".source-term")).toHaveText("reserve");
  await expect(page.locator(".source-term")).not.toContainText("push");
});
