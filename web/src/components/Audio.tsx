import { createEffect, createMemo, createSignal } from "solid-js";
import { postJson } from "../auth";
import { toggleMute } from "../actions";
import { series, showToast, snapshot } from "../store";
import { Sparkline } from "./Sparkline";

const DEBOUNCE_MS = 250;

export function Audio() {
  const [dragging, setDragging] = createSignal(false);
  const [localVolume, setLocalVolume] = createSignal<number | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;

  // Mirror device-side volume into the slider unless the user is mid-drag —
  // a snapshot tick during a drag would jerk the slider back to the
  // persisted value while the operator is still moving it.
  createEffect(() => {
    if (dragging()) return;
    const s = snapshot();
    if (s) setLocalVolume(s.audio.volume_pct);
  });

  const muted = createMemo(() => snapshot()?.audio.muted ?? false);
  const volume = createMemo(() => localVolume() ?? snapshot()?.audio.volume_pct ?? 50);

  const scheduleVolume = (level: number) => {
    if (timer != null) clearTimeout(timer);
    timer = setTimeout(async () => {
      timer = null;
      try {
        await postJson("/volume", { level });
        showToast(`volume → ${level}%`);
      } catch (e) {
        showToast((e as Error).message, true);
      }
    }, DEBOUNCE_MS);
  };

  return (
    <section>
      <h2>Audio</h2>
      <label>
        Volume: <span>{volume()}%</span>
        <input
          type="range"
          min={0}
          max={100}
          step={1}
          value={volume()}
          onPointerDown={() => setDragging(true)}
          onPointerUp={() => setDragging(false)}
          onInput={(e) => {
            const v = Number(e.currentTarget.value);
            setLocalVolume(v);
            scheduleVolume(v);
          }}
        />
      </label>
      <div class="spark-card" style="margin-top:8px">
        <div class="spark-head">
          <span class="spark-label">VOLUME · LAST 2m</span>
          <span class="spark-value">{volume()}%</span>
        </div>
        <Sparkline values={series("audio_volume_pct")} width={300} height={32} min={0} max={100} fill="var(--accent-soft)" />
      </div>
      <div class="btn-row" style="margin-top:8px">
        <button type="button" onClick={toggleMute} data-shortcut="mute">
          {muted() ? "Unmute" : "Mute"}
        </button>
      </div>
      <small>
        Slider POSTs /volume after a brief debounce. Mute is independent of volume so
        unmuting restores the prior level. Both persist to STACKCHAN.RON.
      </small>
    </section>
  );
}
