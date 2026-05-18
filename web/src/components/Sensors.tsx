import { Show, createSignal, onCleanup, onMount } from "solid-js";
import type { SensorsSnapshot } from "../types";
import { showToast } from "../store";

const POLL_MS = 1000;

function fmtVec(v: readonly [number, number, number], digits: number): string {
  return `(${v[0].toFixed(digits)}, ${v[1].toFixed(digits)}, ${v[2].toFixed(digits)})`;
}

function intensityLabel(v: number): string {
  if (v >= 3) return "high";
  if (v === 2) return "mid";
  if (v === 1) return "low";
  return "—";
}

export function Sensors() {
  const [data, setData] = createSignal<SensorsSnapshot | null>(null);
  let timer: ReturnType<typeof setInterval> | null = null;

  const tick = async () => {
    try {
      const res = await fetch("/sensors");
      if (!res.ok) return;
      setData((await res.json()) as SensorsSnapshot);
    } catch (e) {
      showToast(`/sensors: ${(e as Error).message}`, true);
    }
  };

  onMount(() => {
    void tick();
    timer = setInterval(tick, POLL_MS);
  });
  onCleanup(() => {
    if (timer != null) clearInterval(timer);
  });

  return (
    <section>
      <h2>Sensors</h2>
      <Show
        when={data()}
        fallback={
          <div class="empty">
            <div class="empty-title">No sensor reads yet</div>
            <small>/sensors polls every 1 s.</small>
          </div>
        }
      >
        {(s) => (
          <dl class="row">
            <dt>IMU accel (g)</dt>
            <dd>{s().imu ? fmtVec(s().imu!.accel_g, 3) : "—"}</dd>
            <dt>IMU gyro (°/s)</dt>
            <dd>{s().imu ? fmtVec(s().imu!.gyro_dps, 2) : "—"}</dd>
            <dt>Ambient (lux)</dt>
            <dd>{s().ambient_lux != null ? s().ambient_lux!.toFixed(2) : "—"}</dd>
            <dt>Audio RMS</dt>
            <dd>{s().audio_rms.toFixed(4)}</dd>
            <dt>Body touch</dt>
            <dd>
              {s().body_touch
                ? `L:${intensityLabel(s().body_touch!.left)} C:${intensityLabel(s().body_touch!.centre)} R:${intensityLabel(s().body_touch!.right)}`
                : "—"}
            </dd>
          </dl>
        )}
      </Show>
      <small>Polled every 1 s from GET /sensors. Producers mirror into a snapshot static; HTTP never drains the source signals.</small>
    </section>
  );
}
