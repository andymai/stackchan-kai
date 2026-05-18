import { Show, createMemo } from "solid-js";
import { series, snapshot } from "../store";
import { Sparkline } from "./Sparkline";

function SparkCard(props: {
  label: string;
  values: number[];
  value: string;
  min?: number;
  max?: number;
}) {
  return (
    <div class="spark-card">
      <div class="spark-head">
        <span class="spark-label">{props.label}</span>
        <span class="spark-value">{props.value}</span>
      </div>
      <Sparkline
        values={props.values}
        width={220}
        height={36}
        min={props.min}
        max={props.max}
        fill="var(--accent-soft)"
      />
    </div>
  );
}

export function Telemetry() {
  const battery = createMemo(() => snapshot()?.battery.percent);
  const volume = createMemo(() => snapshot()?.audio.volume_pct);
  const pan = createMemo(() => snapshot()?.head_pose.pan_deg);
  const tilt = createMemo(() => snapshot()?.head_pose.tilt_deg);

  return (
    <section>
      <h2>Telemetry</h2>
      <Show
        when={snapshot()}
        fallback={<small>waiting for first SSE sample…</small>}
      >
        <div class="spark-grid">
          <SparkCard
            label="BATTERY · LAST 2m"
            values={series("battery_pct")}
            value={battery() != null ? `${battery()}%` : "—"}
            min={0}
            max={100}
          />
          <SparkCard
            label="VOLUME · LAST 2m"
            values={series("audio_volume_pct")}
            value={`${volume() ?? 0}%`}
            min={0}
            max={100}
          />
          <SparkCard
            label="PAN · LAST 2m"
            values={series("pan_deg")}
            value={`${(pan() ?? 0).toFixed(0)}°`}
            min={-60}
            max={60}
          />
          <SparkCard
            label="TILT · LAST 2m"
            values={series("tilt_deg")}
            value={`${(tilt() ?? 0).toFixed(0)}°`}
            min={-30}
            max={30}
          />
        </div>
      </Show>
      <small>Client-side ring buffer of the last ~2 min of /state SSE samples; resets on reload.</small>
    </section>
  );
}
