import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";

const read = (path) => fs.readFileSync(new URL(`../../${path}`, import.meta.url), "utf8");
const themeCss = read("web/shared/theme.css");
const kidsShell = read("web/alix-kids/kids/shell.css");
const kidsDomCss = read("web/alix-kids/kids/dom.css");
const kidsThemeJs = read("web/alix-kids/kids/theme.js");

const MINIMUM_RATIO = 3;
const ADULT_PALETTES = [":root,\n[data-theme=\"dark\"]", "[data-theme=\"light\"]", "[data-theme=\"sunrise\"]", "[data-theme=\"ocean\"]", "[data-theme=\"berry\"]"];

function rgb(hex) {
  const digits = hex.slice(1);
  const full = digits.length === 3 ? [...digits].map((digit) => digit + digit).join("") : digits;
  return full.match(/../g).map((part) => Number.parseInt(part, 16));
}

function composite(foreground, background, alpha) {
  const front = rgb(foreground);
  const back = rgb(background);
  const mixed = front.map((value, index) => Math.round(value * alpha + back[index] * (1 - alpha)));
  return `#${mixed.map((value) => value.toString(16).padStart(2, "0")).join("")}`;
}

function channel(value) {
  value /= 255;
  return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
}

function luminance(hex) {
  const [red, green, blue] = rgb(hex);
  return 0.2126 * channel(red) + 0.7152 * channel(green) + 0.0722 * channel(blue);
}

function contrast(first, second) {
  const [high, low] = [luminance(first), luminance(second)].sort((a, b) => b - a);
  return (high + 0.05) / (low + 0.05);
}

function block(css, selector) {
  const start = css.indexOf(`${selector} {`);
  assert.notEqual(start, -1, `selector ${JSON.stringify(selector)} exists`);
  return css.slice(start, css.indexOf("}", start));
}

function token(body, name) {
  const match = body.match(new RegExp(`${name}:\\s*(#[0-9a-fA-F]{3,6})\\b`));
  assert.ok(match, `${name} is a hex colour`);
  return match[1];
}

function ruleValue(css, selector) {
  const match = css.match(new RegExp(`${selector.replace(/[.]/g, "\\.")}\\s*\\{\\s*border-color:\\s*([^;}]+);`));
  assert.ok(match, `${selector} declares a border-color`);
  return match[1].trim();
}

function mutedBorder(css, selector) {
  const value = ruleValue(css, selector);
  const mix = value.match(/^color-mix\(in srgb, var\((--[a-z0-9-]+)\) (\d+)%, transparent\)$/);
  if (mix) return { name: mix[1], alpha: Number(mix[2]) / 100 };
  const plain = value.match(/^var\((--[a-z0-9-]+)\)$/);
  assert.ok(plain, `${selector} border is var(--token) or a transparent color-mix, got ${value}`);
  return { name: plain[1], alpha: 1 };
}

function effective(border, background, alpha) {
  return alpha === 1 ? border : composite(border, background, alpha);
}

const ASKED_TODO = "the asked border (--bolt-hi, kids --accent) is below 3:1 in some palettes; fixing it recolours the palette, which needs a ruling";

function adultBorderCases(role, name, alpha) {
  const failures = [];
  for (const selector of ADULT_PALETTES) {
    const palette = block(themeCss, selector);
    const border = token(palette, name);
    for (const background of ["--panel-top", "--panel-bot", "--void"].map((key) => token(palette, key))) {
      const ratio = contrast(effective(border, background, alpha), background);
      if (ratio < MINIMUM_RATIO) {
        failures.push(`${selector.replace(/\s+/g, " ")}: ${role} border ${name} ${border} on ${background} is ${ratio.toFixed(2)}:1`);
      }
    }
  }
  return failures;
}

function kidsAskedFailures() {
  const card = token(block(kidsShell, ":root"), "--card");
  return [...kidsThemeJs.matchAll(/(\w+): \{[^}]*accent: "(#[0-9a-f]{6})"/g)]
    .filter(([, , accent]) => contrast(accent, card) < MINIMUM_RATIO)
    .map(([, name, accent]) => `kids ${name} asked border ${accent} on ${card} is ${contrast(accent, card).toFixed(2)}:1`);
}

test("adult muted blank outline reaches 3:1 against every card background in each palette", () => {
  const muted = mutedBorder(themeCss, ".blank-pill.muted");
  assert.deepEqual(adultBorderCases("muted", muted.name, muted.alpha), []);
});

test("kids muted blank outline reaches 3:1 against the card", () => {
  const card = token(block(kidsShell, ":root"), "--card");
  const muted = mutedBorder(kidsDomCss, ".blank-pill.muted");
  const colour = effective(token(block(kidsShell, ":root"), muted.name), card, muted.alpha);
  assert.ok(contrast(colour, card) >= MINIMUM_RATIO,
    `kids muted border ${colour} on ${card} is ${contrast(colour, card).toFixed(2)}:1`);
});

test("adult asked blank outline reaches 3:1 in each palette", { todo: ASKED_TODO }, () => {
  assert.deepEqual(adultBorderCases("asked", "--bolt-hi", 1), []);
});

test("kids asked blank outline reaches 3:1 in each palette", { todo: ASKED_TODO }, () => {
  assert.deepEqual(kidsAskedFailures(), []);
});
