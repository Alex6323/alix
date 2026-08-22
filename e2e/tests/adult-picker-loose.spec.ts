import { test, expect } from "./helpers";
import { adultDeckRow, openApp } from "./helpers";

test.beforeEach(async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  await openApp(page);
});

test("the picker shows a new loose deck beside workspace groups", async ({ page, request }, testInfo) => {
  const response = await request.get("/api/decks");
  expect(response.ok()).toBe(true);
  const catalog = await response.json();
  expect(catalog.recent.some((row: { label: string }) => row.label === "Loose Facts")).toBe(true);

  await expect(adultDeckRow(page, "Animals")).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath("mixed-root-picker.png"),
    fullPage: true,
    animations: "disabled",
  });

  await expect(adultDeckRow(page, "Loose Facts")).toBeVisible();
  await expect(page.getByText("Decks", { exact: true })).toBeVisible();
});
