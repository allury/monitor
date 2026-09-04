(() => {
  let saved;
  try { saved = localStorage.getItem("monitor-theme"); } catch { /* 存储受限时跟随系统 */ }
  document.documentElement.classList.toggle("dark", saved ? saved === "dark" : matchMedia("(prefers-color-scheme: dark)").matches);
  const label = () => {
    const button = document.querySelector("#theme-toggle");
    if (button) button.setAttribute("aria-label", document.documentElement.classList.contains("dark") ? "切换明亮主题" : "切换暗黑主题");
  };
  document.addEventListener("click", event => {
    if (!event.target.closest?.("#theme-toggle")) return;
    const dark = document.documentElement.classList.toggle("dark");
    try { localStorage.setItem("monitor-theme", dark ? "dark" : "light"); } catch { /* 当前页面仍可切换 */ }
    label();
  });
  document.addEventListener("DOMContentLoaded", label);
  document.addEventListener("monitor:page", label);
})();
