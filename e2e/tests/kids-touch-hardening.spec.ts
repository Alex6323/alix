import { test, expect } from "./helpers";
import { kidsDeckRow, openApp } from "./helpers";

test.use({ hasTouch: true });

test.beforeEach(async ({ request }) => {
  await request.post("/api/deselect", { data: {} });
});

test.afterEach(async ({ request }) => {
  await request.post("/api/deselect", { data: {} });
});

test("a touch option suppresses the browser menu and still reaches choose", async ({ page }) => {
  await openApp(page);
  await page.locator(".box", { hasText: "Animals" }).tap();
  await kidsDeckRow(page, "wild").tap();
  await Promise.all([
    page.waitForResponse((response) => response.url().includes("/api/select")),
    page.getByRole("button", { name: "Tap the answer" }).tap(),
  ]);

  const option = page.locator(".opt-btn").first();
  await expect(option).toBeVisible();
  const touchStyles = await option.evaluate((element) => {
    const styles = getComputedStyle(element.ownerDocument.body);
    return { touchAction: styles.touchAction, userSelect: styles.userSelect };
  });
  expect(touchStyles).toEqual({ touchAction: "manipulation", userSelect: "none" });

  const optionMenuPrevented = await option.evaluate((element) => {
    const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    element.dispatchEvent(event);
    return event.defaultPrevented;
  });
  expect(optionMenuPrevented, "a long press must not open a menu over an answer").toBe(true);

  await page.getByRole("button", { name: "Ask Alix" }).tap();
  const tutorInput = page.locator("#askInput");
  await expect(tutorInput).toBeVisible();
  const inputMenuPrevented = await tutorInput.evaluate((element) => {
    const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    element.dispatchEvent(event);
    return event.defaultPrevented;
  });
  expect(inputMenuPrevented, "the tutor input must keep its paste menu").toBe(false);
  await page.getByRole("button", { name: "Close" }).tap();

  const [chooseResponse] = await Promise.all([
    page.waitForResponse((response) =>
      response.url().includes("/api/choose") && response.request().method() === "POST"
    ),
    option.tap(),
  ]);
  expect(chooseResponse.status()).toBe(200);
  await expect(page.locator(".opt-correct")).toHaveCount(1);
});
