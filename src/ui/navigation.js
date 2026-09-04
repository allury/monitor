import {mountStatus} from "./app.js";
import {mountAdmin} from "./admin.js";

const publicRoute = pathname => pathname === "/" || /^\/node\/[^/]+$/.test(pathname);
const localRoute = url => url.origin === location.origin && (publicRoute(url.pathname) || url.pathname === "/admin");
let currentUrl = location.href, pending = null;
const mount = () => document.querySelector("#home-view") ? mountStatus() : mountAdmin();
let page = mount();

function navigationError() {
  document.querySelector("#navigation-error")?.remove();
  const message = document.createElement("p");
  message.id = "navigation-error"; message.className = "navigation-error";
  message.setAttribute("role", "alert");
  message.textContent = "页面切换失败，当前页面已保留。请再次点击重试。";
  document.body.append(message);
}

async function navigate(url, push = true) {
  pending?.abort();
  const request = new AbortController(); pending = request;
  document.querySelector("#navigation-error")?.remove();
  try {
    if (page.kind === "status" && publicRoute(url.pathname)) {
      page.route(url.pathname);
    } else if (url.pathname !== new URL(currentUrl).pathname) {
      const response = await fetch(url.pathname, {cache:"no-cache", signal:AbortSignal.any([request.signal, AbortSignal.timeout(10000)])});
      if (!response.ok) throw new Error("Navigation failed");
      const html = await response.text();
      if (request !== pending) return;
      const next = new DOMParser().parseFromString(html, "text/html");
      const expected = url.pathname === "/admin" ? "#admin-view" : "#home-view";
      if (!next.querySelector(expected)) throw new Error("Unexpected page");
      // Both surfaces use the same embedded assets. Imported page modules own
      // their timers, requests and credentials; old instances must be disposed.
      next.querySelectorAll("script").forEach(script => script.remove());
      page.dispose();
      document.body.className = next.body.className;
      document.body.replaceChildren(...Array.from(next.body.childNodes, node => document.importNode(node, true)));
      document.title = next.title;
      document.querySelector('meta[name="description"]').content = next.querySelector('meta[name="description"]')?.content || "";
      // Set the URL before mounting so a direct node route initializes correctly.
      if (push && url.href !== location.href) history.pushState(null, "", url.href);
      currentUrl = url.href;
      page = mount();
      document.dispatchEvent(new Event("monitor:page"));
    }
    if (request !== pending) return;
    if (push && url.href !== location.href) history.pushState(null, "", url.href);
    currentUrl = url.href;
    window.scrollTo({top:0,behavior:"instant"});
  } catch (error) {
    if (error.name === "AbortError" || request !== pending) return;
    if (!push) history.replaceState(null, "", currentUrl);
    navigationError();
  }
}

document.addEventListener("click", event => {
  if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
  const link = event.target.closest?.("a[href]");
  if (!link || link.hasAttribute("download") || (link.target && link.target !== "_self")) return;
  const url = new URL(link.href, location.href);
  if (!localRoute(url) || url.hash) return;
  event.preventDefault();
  navigate(url);
});
window.addEventListener("popstate", () => navigate(new URL(location.href), false));
window.addEventListener("pagehide", () => { pending?.abort(); page.dispose(); });
window.addEventListener("pageshow", event => { if (event.persisted) { page = mount(); currentUrl = location.href; } });
