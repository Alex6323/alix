import { test, expect } from "./helpers";
import { kidsDeckRow, openApp } from "./helpers";
import type { Locator, Page } from "@playwright/test";

const IMAGE =
  "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='100' height='100'%3E%3Crect width='100' height='100' fill='white'/%3E%3C/svg%3E";

const regions = [
  { role: "asked", reveal_on_answer: true, x: 5, y: 5, width: 20, height: 20, unit: "%" },
  { role: "mask", reveal_on_answer: false, x: 35, y: 5, width: 20, height: 20, unit: "%" },
  { role: "cover", reveal_on_answer: false, x: 65, y: 5, width: 20, height: 20, unit: "%" },
  { role: "mask", reveal_on_answer: false, x: 90, y: 70, width: 20, height: 20, unit: "%" },
];

function state(imageRegions = regions, crop = null) {
  return {
    kind: "review",
    study_revision: 1,
    phase: "review",
    card: {
      id: "kids-mask-card",
      front: "Name the marked part",
      front_runs: [{ text: "Name the marked part" }],
      front_units: null,
      context: [],
      context_runs: [],
      back: ["answer"],
      back_runs: [[{ text: "answer" }]],
      back_units: [{ kind: "sentence", text: "answer" }],
      reshaped: false,
      note: [],
      images: [{ src: IMAGE, alt: "kids mask geometry probe", regions: imageRegions, crop }],
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
    label: "kids-image-mask-review.md",
    save_error: null,
  };
}

async function openSyntheticCard(page: Page, imageRegions = regions, crop = null) {
  await page.route("**/api/select", (route) => route.fulfill({ json: state(imageRegions, crop) }));
  await openApp(page);
  await page.locator(".box", { hasText: "Animals" }).click();
  await kidsDeckRow(page, "wild").click();
  await page.getByRole("button", { name: "Say it yourself" }).click();
  await expect(page.locator(".rev-img-mask")).toHaveCount(imageRegions.length);
}

async function visualSignature(locator: Locator) {
  return locator.evaluate((node) => {
    const style = getComputedStyle(node);
    return {
      glyph: node.textContent,
      background: style.backgroundColor,
      border: style.border,
      opacity: style.opacity,
    };
  });
}

test("asked, sibling, and cover masks have three distinct visual treatments", async ({ page }) => {
  await openSyntheticCard(page);
  const byRole = (role: string) => page.locator(`.rev-img-mask[data-region*='"role":"${role}"']`).first();

  const asked = await visualSignature(byRole("asked"));
  const sibling = await visualSignature(byRole("mask"));
  const cover = await visualSignature(byRole("cover"));

  expect(asked).not.toEqual(sibling);
  expect(asked).not.toEqual(cover);
  expect(sibling, "a sibling mask and a cover must remain visually distinguishable").not.toEqual(cover);
});

test("a valid region extending past the source edge is clipped to the image", async ({ page }) => {
  await openSyntheticCard(page);
  const image = await page.locator("img[alt='kids mask geometry probe']").boundingBox();
  const region = await page.locator(`.rev-img-mask[data-region*='"x":90']`).boundingBox();
  expect(image).not.toBeNull();
  expect(region).not.toBeNull();
  expect(region!.x + region!.width, "the mask must not paint to the right of the source").toBeLessThanOrEqual(
    image!.x + image!.width + 0.5,
  );
});

test("the cropped-image path clips a pixel region at the source edge", async ({ page }) => {
  await openSyntheticCard(
    page,
    [{ role: "asked", reveal_on_answer: true, x: 90, y: 10, width: 20, height: 20, unit: "px" }],
    { x: 80, y: 0, width: 40, height: 100, unit: "px" },
  );
  const image = await page.locator("img[alt='kids mask geometry probe']").boundingBox();
  const region = await page.locator(".rev-img-mask").boundingBox();
  expect(image).not.toBeNull();
  expect(region).not.toBeNull();
  expect(region!.x + region!.width, "the mask must not paint to the right of the source").toBeLessThanOrEqual(
    image!.x + image!.width + 0.5,
  );
});

test("an asked pixel region wholly outside the source fails loudly", async ({ page }) => {
  await openSyntheticCard(page, [
    { role: "asked", reveal_on_answer: true, x: 120, y: 10, width: 20, height: 20, unit: "px" },
  ]);

  await expect(
    page.locator("#oops"),
    "ADR 0034 requires a loud render failure instead of silently exposing an unmasked question",
  ).toBeVisible();
});
