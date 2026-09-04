(() => {
  let saved;
  try { saved = localStorage.getItem("monitor-theme"); } catch { /* 存储受限时跟随系统 */ }
  document.documentElement.classList.toggle("dark", saved ? saved === "dark" : matchMedia("(prefers-color-scheme: dark)").matches);
  document.addEventListener("DOMContentLoaded", () => {
    document.querySelector("#theme-toggle")?.addEventListener("click", () => {
      const dark = document.documentElement.classList.toggle("dark");
      try { localStorage.setItem("monitor-theme", dark ? "dark" : "light"); } catch { /* 当前页面仍可切换 */ }
    });
  });
})();
