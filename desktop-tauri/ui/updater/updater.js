// 更新のウィンドウの表示。Rust 側 (src-tauri/src/update.rs の UpdaterWindow) が
// window.zooUpdater.set({ title, detail, progress, busy, tone }) を呼んで状態を渡す。
// progress: null なら進み具合の分からないバー、0〜1 ならその割合、"hidden" ならバーを隠す。
(function () {
  const root = document.querySelector(".updater");
  const title = document.getElementById("title");
  const detail = document.getElementById("detail");
  const bar = document.getElementById("bar");
  const fill = document.getElementById("fill");

  function set(state) {
    title.textContent = state.title ?? "";
    detail.textContent = state.detail ?? "";
    root.dataset.busy = state.busy ? "true" : "false";
    root.dataset.tone = state.tone ?? "normal";
    if (state.progress === "hidden") {
      bar.dataset.state = "hidden";
      bar.removeAttribute("aria-valuenow");
    } else if (typeof state.progress === "number") {
      const percent = Math.max(0, Math.min(1, state.progress)) * 100;
      bar.dataset.state = "determinate";
      fill.style.width = `${percent}%`;
      bar.setAttribute("aria-valuenow", String(Math.round(percent)));
    } else {
      bar.dataset.state = "indeterminate";
      fill.style.width = "";
      bar.removeAttribute("aria-valuenow");
    }
  }

  window.zooUpdater = { set };
})();
