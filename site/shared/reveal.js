// Reveal: elements with .reveal fade up the first time they scroll into view. Without JS they're simply shown.
document.documentElement.classList.replace("no-js", "js");
const io = new IntersectionObserver((es) => { for (const e of es) if (e.isIntersecting) { e.target.classList.add("is-in"); io.unobserve(e.target); } }, { rootMargin: "0px 0px -8% 0px" });
document.querySelectorAll(".reveal").forEach((el) => io.observe(el));
