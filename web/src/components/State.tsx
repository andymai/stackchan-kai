import { Show } from "solid-js";
import { snapshot } from "../store";

function Skeleton(props: { width?: string }) {
  return <span class="skeleton skeleton-row" style={props.width ? `min-width:${props.width}` : ""}>—</span>;
}

export function State() {
  return (
    <section aria-label="State">
      <h2>State</h2>
      <dl class="row">
        <dt>Emotion</dt>
        <dd>
          <Show when={snapshot()?.emotion} fallback={<Skeleton width="6ch" />}>
            {(e) => e()}
          </Show>
        </dd>
        <dt>Mood</dt>
        <dd>
          <Show when={snapshot()?.mood} fallback={<Skeleton width="6ch" />}>
            {(m) => m()}
          </Show>
        </dd>
        <dt>Decorator</dt>
        <dd>{snapshot()?.decorator ?? <small style="opacity:0.7">none</small>}</dd>
        <dt>Head pose</dt>
        <dd>
          <Show when={snapshot()} fallback={<Skeleton width="14ch" />}>
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
          <Show when={snapshot()} fallback={<Skeleton width="8ch" />}>
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
          <Show when={snapshot()} fallback={<Skeleton width="10ch" />}>
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
