// The site is light paper for everyone. A visitor can switch to dark paper with the footer button; the choice is kept
// in this browser's local storage and never leaves it. Loaded in <head>, before the page paints, so there's no flash.
(() => {
  const root = document.documentElement;
  const key = "emberhut-theme";
  let dark = false;
  try { dark = localStorage.getItem(key) === "dark"; } catch { /* storage blocked: stay light */ }
  function apply() {
    root.dataset.theme = dark ? "dark" : "light";
    root.style.colorScheme = dark ? "dark" : "light";
    const meta = document.querySelector('meta[name="theme-color"]');
    if (meta) meta.content = dark ? "#17120e" : "#f1e3cb";
    for (const b of document.querySelectorAll("[data-theme-toggle]")) {
      b.setAttribute("aria-pressed", String(dark));
      b.textContent = dark ? "Light paper" : "Dark paper";
    }
  }
  apply();
  document.addEventListener("DOMContentLoaded", apply);
  document.addEventListener("click", (e) => {
    if (!(e.target instanceof Element) || !e.target.closest("[data-theme-toggle]")) return;
    dark = !dark;
    try { localStorage.setItem(key, dark ? "dark" : "light"); } catch { /* not kept, still switched */ }
    apply();
  });
})();
