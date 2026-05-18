import { For } from "solid-js";
import { toasts } from "../store";

export function Toast() {
  return (
    <div class="toast-stack" role="status" aria-live="polite" aria-atomic="false">
      <For each={toasts()}>
        {(t) => (
          <div class={`toast show${t.bad ? " bad" : ""}`} role={t.bad ? "alert" : undefined}>
            {t.msg}
          </div>
        )}
      </For>
    </div>
  );
}
