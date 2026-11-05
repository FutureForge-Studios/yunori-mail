// Applies the saved (or system) theme before first paint — an external file,
// not an inline script, so the server's CSP can stay `script-src 'self'`.
(function () {
  try {
    var t = localStorage.getItem("theme") || "system";
    var dark = t === "dark" || (t === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
    if (dark) document.documentElement.classList.add("dark");
  } catch (e) {}
})();
