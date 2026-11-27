// /pair: a pairing link (https://emberhut.com/pair#id=…&code=…&route=…) opened in a browser instead of the app.
// Everything is in the fragment, which the browser never sends to a server; this page reads it here, shows the
// code to type into the app, and makes no requests.
const f = new URLSearchParams(location.hash.slice(1));
const code = (f.get("code") || "").trim().toUpperCase();
const ok = /^[A-Z0-9]{4}[- ]?[A-Z0-9]{4}$/.test(code);
const box = document.querySelector("[data-code]");
const none = document.querySelector("[data-nocode]");
if (box && ok) {
  box.textContent = code.length === 8 ? code.slice(0, 4) + "-" + code.slice(4) : code.replace(" ", "-");
  box.closest("[data-has-code]")?.removeAttribute("hidden");
} else if (none) none.hidden = false;
