import { test, expect } from "./helpers";
import { adultDeckRow, openApp } from "./helpers";

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await openApp(page);
});

test("fact citations use the editor-style source panel", async ({ page }) => {
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "Source Fact").click();
  await page.getByTitle("choose a depth").click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: /^Recall/ }).click(),
  ]);

  await page.getByRole("button", { name: "Reveal" }).click();
  await page.keyboard.press("s");

  const panel = page.locator(".source-excerpt");
  await expect(panel).toBeVisible();
  await expect(panel.locator(".source-file")).toContainText("source-fact.rs");
  await expect(panel.locator(".source-line")).toHaveCount(4);
  await expect(panel.locator(".source-number")).toHaveText(["1", "2", "3", "4"]);
  await expect(panel.locator(".source-text").nth(1)).toContainText("reserve(1)");
  await expect(panel.locator(".source-term")).toHaveCount(0);
  await expect(page.locator(".excerpt, .wexcerpt")).toHaveCount(0);
});
