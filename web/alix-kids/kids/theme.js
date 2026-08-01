const KIDS_THEMES = {
  Sunrise: { bgTop: "#fff5ec", bgBot: "#ffe7d4", accent: "#ff8a3d", shadow: "#e0702a" },
  Ocean: { bgTop: "#eafaf7", bgBot: "#cdeeff", accent: "#0fa8b4", shadow: "#0b7d86" },
  Berry: { bgTop: "#fdeefb", bgBot: "#f1dbff", accent: "#c04bd0", shadow: "#9c34ab" },
};

export function createKidsTheme({ storage, rootStyle }) {
let selected = loadTheme();

function hasKidsTheme(name) {
  return Object.hasOwn(KIDS_THEMES, name);
}

function loadTheme() {
  try {
    const stored = storage.getItem("alix-kids-theme");
    return hasKidsTheme(stored) ? stored : "Sunrise";
  } catch (error) {
    return "Sunrise";
  }
}

function applyKidsTheme() {
  const value = KIDS_THEMES[selected];
  rootStyle.setProperty("--bg-top", value.bgTop);
  rootStyle.setProperty("--bg-bot", value.bgBot);
  rootStyle.setProperty("--bg", "linear-gradient(168deg, " + value.bgTop + " 0%, " + value.bgBot + " 100%)");
  rootStyle.setProperty("--accent", value.accent);
  rootStyle.setProperty("--accent-sh", value.shadow);
}

function currentKidsTheme() {
  return selected;
}

function kidsThemeNames() {
  return Object.keys(KIDS_THEMES);
}

function kidsThemePalette(name) {
  return hasKidsTheme(name) ? KIDS_THEMES[name] : null;
}

function setKidsTheme(name) {
  if (!hasKidsTheme(name)) return false;
  selected = name;
  try {
    storage.setItem("alix-kids-theme", name);
  } catch (error) {
    // Persistence is optional; the live theme still changes.
  }
  applyKidsTheme();
  return true;
}

return {
  apply: applyKidsTheme,
  current: currentKidsTheme,
  names: kidsThemeNames,
  palette: kidsThemePalette,
  set: setKidsTheme,
};
}
