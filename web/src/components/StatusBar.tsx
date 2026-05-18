import { Show } from "solid-js";
import { snapshot } from "../store";
import { FaceGlyph } from "./FaceGlyph";

function Stat(props: { label: string; value: string; tone?: "ok" | "warn" | "bad" }) {
  return (
    <div class="stat" classList={{ "is-ok": props.tone === "ok", "is-warn": props.tone === "warn", "is-bad": props.tone === "bad" }}>
      <div class="stat-label">{props.label}</div>
      <div class="stat-value">{props.value}</div>
    </div>
  );
}

function batteryTone(pct: number | null): "ok" | "warn" | "bad" | undefined {
  if (pct == null) return undefined;
  if (pct >= 40) return "ok";
  if (pct >= 15) return "warn";
  return "bad";
}

export function StatusBar() {
  return (
    <header class="status-bar">
      <div class="status-face">
        <FaceGlyph size={56} />
      </div>
      <div class="status-stats">
        <Stat label="EMOTION" value={snapshot()?.emotion?.toUpperCase() ?? "—"} />
        <Stat label="MOOD" value={snapshot()?.mood?.toUpperCase() ?? "—"} />
        <Show
          when={snapshot()}
          fallback={<Stat label="POSE" value="—" />}
        >
          {(s) => {
            const p = s().head_pose;
            return <Stat label="POSE" value={`${p.pan_deg.toFixed(0)}° / ${p.tilt_deg.toFixed(0)}°`} />;
          }}
        </Show>
        <Show
          when={snapshot()}
          fallback={<Stat label="BATTERY" value="—" />}
        >
          {(s) => {
            const b = s().battery;
            const v = b.percent != null ? `${b.percent}%` : "—";
            return <Stat label="BATTERY" value={v} tone={batteryTone(b.percent)} />;
          }}
        </Show>
        <Show
          when={snapshot()}
          fallback={<Stat label="WIFI" value="—" />}
        >
          {(s) => {
            const w = s().wifi;
            return (
              <Stat
                label="WIFI"
                value={w.connected ? w.ip ?? "up" : "down"}
                tone={w.connected ? "ok" : "bad"}
              />
            );
          }}
        </Show>
        <Show
          when={snapshot()}
          fallback={<Stat label="AUDIO" value="—" />}
        >
          {(s) => {
            const a = s().audio;
            return (
              <Stat
                label="AUDIO"
                value={a.muted ? "MUTED" : `${a.volume_pct}%`}
                tone={a.muted ? "warn" : undefined}
              />
            );
          }}
        </Show>
      </div>
    </header>
  );
}
