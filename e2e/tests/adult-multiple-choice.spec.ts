import { test, expect, openApp } from "./helpers";

const DECK = "animals/multiple.md";
const SEQUENCE_DECK = "animals/multiple-sequence.md";

test("adult select-all toggles independently and submits the exact set", async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  const reset = await request.post("/api/reset", { data: { deck: DECK } });
  expect(reset.ok(), await reset.text()).toBeTruthy();

  const select = await request.post("/api/select", {
    data: { deck: DECK, depth: "recognize", session: 1 },
  });
  expect(select.ok(), await select.text()).toBeTruthy();
  const introduction = await select.json();
  expect(introduction.choices_multiple).toBe(true);
  expect(introduction.introducing).toBe(true);

  await openApp(page);
  await expect(page.locator(".front-text")).toHaveText("Which animals are mammals?");

  const option = (text: string) => page.locator(".option").filter({
    has: page.locator(".opt", { hasText: text, exact: true }),
  });
  const chosenIndices = () => page.locator(".option").evaluateAll((rows) => rows.flatMap(
    (row, index) => row.getAttribute("aria-pressed") === "true" ? [index] : [],
  ));
  const chooseRequests: string[] = [];
  page.on("request", (outgoing) => {
    if (outgoing.url().includes("/api/choose")) chooseRequests.push(outgoing.url());
  });

  await option("Dolphin").click();
  await expect(option("Dolphin")).toHaveAttribute("aria-pressed", "true");
  await expect(option("Otter")).toHaveAttribute("aria-pressed", "false");
  await option("Otter").click();
  await option("Dolphin").click();
  await expect(option("Dolphin")).toHaveAttribute("aria-pressed", "false");
  await expect(option("Otter")).toHaveAttribute("aria-pressed", "true");
  await option("Dolphin").click();
  await expect(option("Dolphin")).toHaveAttribute("aria-pressed", "true");
  expect(chooseRequests, "toggling must stay local until Submit").toEqual([]);

  const exactIndices = await chosenIndices();
  const [exactRequest, exactResponse] = await Promise.all([
    page.waitForRequest((outgoing) => outgoing.url().includes("/api/choose")),
    page.waitForResponse((incoming) => incoming.url().includes("/api/choose")),
    page.getByRole("button", { name: "Submit" }).click(),
  ]);
  expect(exactRequest.postDataJSON()).toEqual({
    indices: exactIndices,
    card: introduction.card.id,
  });
  const exactFeedback = await exactResponse.json();
  expect(exactFeedback.passed).toBe(true);
  expect(exactFeedback.chosen).toEqual(exactIndices);
  await expect(option("Dolphin")).toHaveClass(/correct/);
  await expect(option("Dolphin").locator(".choice-status")).toHaveText("chosen · correct");
  await expect(option("Otter")).toHaveClass(/correct/);
  await expect(option("Otter").locator(".choice-status")).toHaveText("chosen · correct");
  await expect(option("Trout")).toHaveClass(/dim/);
  await expect(option("Lizard")).toHaveClass(/dim/);

  await Promise.all([
    page.waitForResponse((incoming) => incoming.url().includes("/api/introduce")),
    page.getByRole("button", { name: "Seen" }).click(),
  ]);

  await request.post("/api/deselect", { data: {} });
  const retry = await request.post("/api/select", {
    data: { deck: DECK, depth: "recognize", cram: true, session: 1 },
  });
  expect(retry.ok(), await retry.text()).toBeTruthy();
  const review = await retry.json();
  expect(review.choices_multiple).toBe(true);
  expect(review.introducing).toBe(false);

  await openApp(page);
  await option("Dolphin").click();
  await option("Trout").click();
  const partialIndices = await chosenIndices();
  const [partialRequest, partialResponse] = await Promise.all([
    page.waitForRequest((outgoing) => outgoing.url().includes("/api/choose")),
    page.waitForResponse((incoming) => incoming.url().includes("/api/choose")),
    page.getByRole("button", { name: "Submit" }).click(),
  ]);
  expect(partialRequest.postDataJSON()).toEqual({
    indices: partialIndices,
    card: review.card.id,
  });
  const partialFeedback = await partialResponse.json();
  expect(partialFeedback.passed).toBe(false);
  expect(partialFeedback.chosen).toEqual(partialIndices);
  await expect(option("Dolphin")).toHaveClass(/correct/);
  await expect(option("Dolphin").locator(".choice-status")).toHaveText("chosen · correct");
  await expect(option("Trout")).toHaveClass(/wrong/);
  await expect(option("Trout").locator(".choice-status")).toHaveText("chosen · incorrect");
  await expect(option("Otter")).toHaveClass(/correct/);
  await expect(option("Otter").locator(".choice-status")).toHaveText("correct");
  await expect(option("Lizard")).toHaveClass(/dim/);
  await expect(page.getByRole("button", { name: "Continue" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Next" })).toHaveCount(0);
});

test("adult select-all keyboard choices reset focus on the next card", async ({ page, request }) => {
  await request.post("/api/deselect", { data: {} });
  const reset = await request.post("/api/reset", { data: { deck: SEQUENCE_DECK } });
  expect(reset.ok(), await reset.text()).toBeTruthy();

  const select = await request.post("/api/select", {
    data: { deck: SEQUENCE_DECK, depth: "recognize", session: 2 },
  });
  expect(select.ok(), await select.text()).toBeTruthy();
  const introduction = await select.json();
  expect(introduction.choices_multiple).toBe(true);

  await openApp(page);
  await expect(page.locator(".front-text")).toHaveText("Which animals are mammals?");

  const options = page.locator(".option");
  const chooseRequests: string[] = [];
  page.on("request", (outgoing) => {
    if (outgoing.url().includes("/api/choose")) chooseRequests.push(outgoing.url());
  });

  await expect(page.locator(".option.focused .num")).toHaveText("1");
  await page.keyboard.press("Space");
  await expect(options.nth(0)).toHaveAttribute("aria-pressed", "true");
  await page.keyboard.press("3");
  await expect(options.nth(2)).toHaveAttribute("aria-pressed", "true");
  expect(chooseRequests, "Space and digit toggles must stay local until Enter").toEqual([]);

  const [chooseRequest, chooseResponse] = await Promise.all([
    page.waitForRequest((outgoing) => outgoing.url().includes("/api/choose")),
    page.waitForResponse((incoming) => incoming.url().includes("/api/choose")),
    page.keyboard.press("Enter"),
  ]);
  expect(chooseRequest.postDataJSON()).toEqual({
    indices: [0, 2],
    card: introduction.card.id,
  });
  expect((await chooseResponse.json()).chosen).toEqual([0, 2]);

  await Promise.all([
    page.waitForResponse((incoming) => incoming.url().includes("/api/introduce")),
    page.keyboard.press("Enter"),
  ]);
  await expect(page.locator(".front-text")).toHaveText("Which habitats are aquatic?");
  await expect(page.locator(".option.focused .num")).toHaveText("1");
});
