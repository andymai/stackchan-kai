import { Show } from "solid-js";
import { toast } from "../store";

export function Toast() {
  return (
    <Show when={toast()}>
      {(t) => (
        <div class={`toast show${t().bad ? " bad" : ""}`} role="status">
          {t().msg}
        </div>
      )}
    </Show>
  );
}
