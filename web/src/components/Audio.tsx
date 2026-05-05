import { createEffect, createMemo, createSignal } from "solid-js";
import { postJson } from "../auth";
import { showToast, snapshot } from "../store";

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

  const toggleMute = async () => {
    const next = !muted();
    try {
      await postJson("/mute", { muted: next });
      showToast(next ? "muted" : "unmuted");
    } catch (e) {
      showToast((e as Error).message, true);
    }
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
      <div class="btn-row">
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
