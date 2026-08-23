import { test, expect } from "./helpers";
import { kidsDeckRow, openApp } from "./helpers";

const DECK = "animals/multiple.md";

// A select-all card on the kids surface: taps toggle locally, one big Done
// action submits the exact set, and the reply marks every option. Asserts the
// full chain (tap → no request; Done → {indices, card} → feedback classes),
// never just the screen, per this suite's charter.
test("kids select-all toggles locally and submits the exact set on Done", async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  const reset = await request.post("/api/reset", { data: { deck: DECK } });
  expect(reset.ok(), await reset.text()).toBeTruthy();

  await page.setViewportSize({ width: 390, height: 844 });
  await openApp(page);
  await page.locator(".box", { hasText: "Animals" }).click();
  await kidsDeckRow(page, "Select all").click();
  const [, selectResponse] = await Promise.all([
    page.waitForRequest((req) => req.url().includes("/api/select") && req.method() === "POST"),
    page.waitForResponse((res) => res.url().includes("/api/select")),
    page.getByRole("button", { name: "Tap the answer" }).click(),
  ]);
  expect(selectResponse.status(), await selectResponse.text().catch(() => "")).toBe(200);
  const introduction = await selectResponse.json();
  expect(introduction.choices_multiple).toBe(true);

  const option = (text: string) => page.locator(".opt-btn").filter({ hasText: text });
  const chosenIndices = () => page.locator(".opt-btn").evaluateAll((buttons) => buttons.flatMap(
    (button, index) => button.getAttribute("aria-pressed") === "true" ? [index] : [],
  ));
  const chooseRequests: string[] = [];
  page.on("request", (outgoing) => {
    if (outgoing.url().includes("/api/choose")) chooseRequests.push(outgoing.url());
  });

  await option("Dolphin").click();
  await expect(option("Dolphin")).toHaveAttribute("aria-pressed", "true");
  await option("Otter").click();
  await option("Otter").click();
  await expect(option("Otter")).toHaveAttribute("aria-pressed", "false");
  await option("Otter").click();
  await expect(option("Otter")).toHaveAttribute("aria-pressed", "true");
  expect(chooseRequests, "taps must stay local until Done").toEqual([]);

  const exactIndices = await chosenIndices();
  const [exactRequest, exactResponse] = await Promise.all([
    page.waitForRequest((outgoing) => outgoing.url().includes("/api/choose")),
    page.waitForResponse((incoming) => incoming.url().includes("/api/choose")),
    page.getByRole("button", { name: /Done/ }).click(),
  ]);
  expect(exactRequest.postDataJSON()).toEqual({
    indices: exactIndices,
    card: introduction.card.id,
  });
  const feedback = await exactResponse.json();
  expect(feedback.passed).toBe(true);
  expect(feedback.chosen).toEqual(exactIndices);
  await expect(option("Dolphin")).toHaveClass(/opt-correct/);
  await expect(option("Otter")).toHaveClass(/opt-correct/);
  await expect(option("Trout")).toHaveClass(/opt-dim/);
  await expect(option("Lizard")).toHaveClass(/opt-dim/);
  await expect(page.getByRole("button", { name: /Got it! Next/ })).toBeVisible();
});
