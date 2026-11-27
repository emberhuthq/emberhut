// Copy buttons for commands: any [data-copy] button copies the text of the <code> it sits beside (or its
// data-copy value). Without JavaScript the buttons stay hidden and the commands stay selectable.
for (const b of document.querySelectorAll("[data-copy]")) {
  b.hidden = false;
  b.addEventListener("click", async () => {
    const text = b.dataset.copy || b.closest("[data-copy-wrap]")?.querySelector("code")?.innerText || "";
    try { await navigator.clipboard.writeText(text.trim()); b.dataset.done = ""; b.textContent = "Copied"; }
    catch { b.textContent = "Select and copy"; }
    setTimeout(() => { delete b.dataset.done; b.textContent = "Copy"; }, 1800);
  });
}
// "Copy all" in a terminal: only the command lines (after "$ "), not the comments.
for (const b of document.querySelectorAll(".term-copy")) {
  const lines = [...(b.closest("[data-copy-wrap]")?.querySelectorAll("code") || [])].map((c) => c.innerText).join("\n").split("\n");
  const out = []; let more = false;
  for (const l of lines) {
    if (l.startsWith("$ ")) { out.push(l.slice(2)); more = l.trimEnd().endsWith("\\"); }
    else if (more) { out.push(l); more = l.trimEnd().endsWith("\\"); }
  }
  b.dataset.copy = out.join("\n");
}
