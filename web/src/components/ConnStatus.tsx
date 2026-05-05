import { Match, Switch } from "solid-js";
import { conn } from "../store";

export function ConnStatus() {
  return (
    <div class="status">
      <Switch>
        <Match when={conn() === "ok"}>
          <span class="dot ok" />
          <span>live</span>
        </Match>
        <Match when={conn() === "bad"}>
          <span class="dot bad" />
          <span>disconnected</span>
        </Match>
        <Match when={conn() === "connecting"}>
          <span class="dot" />
          <span>connecting…</span>
        </Match>
      </Switch>
    </div>
  );
}
