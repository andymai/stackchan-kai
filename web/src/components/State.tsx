import { Show } from "solid-js";
import { snapshot } from "../store";

export function State() {
  return (
    <section>
      <h2>State</h2>
      <dl class="row">
        <dt>Emotion</dt>
        <dd>{snapshot()?.emotion ?? "—"}</dd>
        <dt>Decorator</dt>
        <dd>{snapshot()?.decorator ?? "—"}</dd>
        <dt>Head pose</dt>
        <dd>
          <Show when={snapshot()} fallback="—">
            {(s) => {
              const p = s().head_pose;
              const a = s().head_actual;
              const base = `pan ${p.pan_deg.toFixed(1)}°, tilt ${p.tilt_deg.toFixed(1)}°`;
              return a
                ? `${base} (actual: ${a.pan_deg.toFixed(1)}°, ${a.tilt_deg.toFixed(1)}°)`
                : base;
            }}
          </Show>
        </dd>
        <dt>Battery</dt>
        <dd>
          <Show when={snapshot()} fallback="—">
            {(s) => {
              const b = s().battery;
              const pct = b.percent != null ? `${b.percent}%` : "—";
              const v = b.voltage_mv != null ? ` (${b.voltage_mv} mV)` : "";
              return `${pct}${v}`;
            }}
          </Show>
        </dd>
        <dt>Wi-Fi</dt>
        <dd>
          <Show when={snapshot()} fallback="—">
            {(s) => {
              const w = s().wifi;
              return w.connected ? `up @ ${w.ip ?? "—"}` : "down";
            }}
          </Show>
        </dd>
      </dl>
    </section>
  );
}
