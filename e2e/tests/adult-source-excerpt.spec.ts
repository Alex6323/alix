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
  const answer = page.locator("#ansRegion .answer");
  await expect(answer).toHaveCount(1);
  await expect(answer).toHaveText(
    "reserve ensures the vector has capacity before the value is inserted.",
  );
  await expect(page.locator("#ansRegion")).toHaveClass(/balanced/);
  await page.keyboard.press("s");

  const panels = page.locator(".source-stack .source-excerpt");
  await expect(panels).toHaveCount(2);
  await expect(panels.nth(0).locator(".source-file")).toContainText("source-fact.rs");
  await expect(panels.nth(0).locator(".source-line")).toHaveCount(4);
  await expect(panels.nth(0).locator(".source-number")).toHaveText(["1", "2", "3", "4"]);
  await expect(panels.nth(0).locator(".source-text").nth(1)).toContainText("reserve(1)");
  await expect(panels.nth(1).locator(".source-number")).toHaveText(["6", "7", "8"]);
  await expect(panels.locator(".source-term")).toHaveCount(0);
  await expect(page.locator(".excerpt, .wexcerpt")).toHaveCount(0);
});

test("visible source scrollbars use the active theme", async ({ page }) => {
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "Source Fact").click();
  await page.getByTitle("choose a depth").click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: /^Recall/ }).click(),
  ]);

  await page.getByRole("button", { name: "Reveal" }).click();
  await page.keyboard.press("s");
  await page.addStyleTag({ content: ".source-text { min-width: 1200px; }" });

  const code = page.locator(".source-code").first();
  await expect(code).toBeVisible();
  const styles = await code.evaluate((element) => ({
    overflows: element.scrollWidth > element.clientWidth,
    height: getComputedStyle(element, "::-webkit-scrollbar").height,
    track: getComputedStyle(element, "::-webkit-scrollbar-track").backgroundColor,
    thumb: getComputedStyle(element, "::-webkit-scrollbar-thumb").backgroundColor,
  }));
  expect(styles.overflows).toBe(true);
  expect(styles.height).toBe("8px");
  expect(styles.track).toBe("rgba(0, 0, 0, 0)");
  expect(styles.thumb).not.toBe("rgba(0, 0, 0, 0)");
});

test("a stale source range warns instead of showing unrelated lines", async ({ page }) => {
  await adultDeckRow(page, "Animals").click();
  await adultDeckRow(page, "Source Stale").click();
  await page.getByTitle("choose a depth").click();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: /^Recall/ }).click(),
  ]);

  await page.getByRole("button", { name: "Reveal" }).click();
  await page.keyboard.press("s");

  const warning = page.locator(".source-stack .cite-err");
  await expect(warning).toBeVisible();
  await expect(warning).toContainText("⚠ 1-3");
  await expect(warning).toContainText("moved to `5-7`");
  await expect(page.locator(".source-stack .source-excerpt")).toHaveCount(0);
  await expect(page.locator(".source-stack")).not.toContainText("wrong()");
});
