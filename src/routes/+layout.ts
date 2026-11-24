/**
 * 关闭 SSR。
 *
 * 这是 Tauri 桌面应用，组件初始化阶段就调用 `getCurrentWindow()`——
 * 它读 `window.__TAURI_INTERNALS__.metadata`，只在 WebView 里存在。
 * SSR 渲染到 Node 时会直接抛 `TypeError: Cannot read properties of undefined`，
 * 症状是 `npm run tauri:dev` 得到一个空白窗口、dev server 只回 500。
 *
 * 对一个没有 SEO、没有首屏数据的本地 SPA，SSR 没有任何收益。
 * 关掉后只做客户端渲染，adapter-static 的 fallback index.html 接管全部路由。
 */
export const ssr = false;
