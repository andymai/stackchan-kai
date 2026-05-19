import { Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { authedFetch, postJson } from "../auth";
import { showToast, snapshot } from "../store";
import { TRACKER_DEFAULT } from "../types";
import type { HeadOffsets, Settings as SettingsType, Tracker } from "../types";

const FRAME_WIDTH = 320;
const FRAME_HEIGHT = 240;

const HEAD_OFFSETS_DEFAULT: HeadOffsets = {
  yaw_offset_deg: 0,
  tilt_offset_deg: 0,
};

// Mirrors the firmware's `Aw88298` 200-ms render-stall budget per
// capture. 2 s gives the avatar comfortable breathing room while
// still tracking the operator's reference target as they move it.
const AUTO_CAPTURE_INTERVAL_MS = 2000;

function rgb565beToImageData(buf: Uint8Array): ImageData {
  const out = new ImageData(FRAME_WIDTH, FRAME_HEIGHT);
  const dst = out.data;
  const view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  for (let i = 0; i < FRAME_WIDTH * FRAME_HEIGHT; i++) {
    const px = view.getUint16(i * 2, false);
    const r5 = (px >> 11) & 0x1f;
    const g6 = (px >> 5) & 0x3f;
    const b5 = px & 0x1f;
    dst[i * 4] = (r5 << 3) | (r5 >> 2);
    dst[i * 4 + 1] = (g6 << 2) | (g6 >> 4);
    dst[i * 4 + 2] = (b5 << 3) | (b5 >> 2);
    dst[i * 4 + 3] = 255;
  }
  return out;
}

export function Calibration() {
  const [tracker, setTracker] = createSignal<Tracker>(TRACKER_DEFAULT);
  const [offsets, setOffsets] = createSignal<HeadOffsets>(HEAD_OFFSETS_DEFAULT);
  const [autoCapture, setAutoCapture] = createSignal(false);
  const [hasSnapshot, setHasSnapshot] = createSignal(false);
  let canvasRef: HTMLCanvasElement | undefined;
  let captureTimer: ReturnType<typeof setInterval> | null = null;

  const loadSettings = async () => {
    try {
      const res = await fetch("/settings");
      if (!res.ok) {
        if (res.status === 503) {
          showToast("calibration unavailable (no SD card)", true);
        } else {
          showToast(`GET /settings: ${res.status}`, true);
        }
        return;
      }
      const c = (await res.json()) as SettingsType;
      setTracker(c.tracker);
    } catch (e) {
      showToast(e instanceof Error ? e.message : String(e), true);
    }
  };

  const loadOffsets = async () => {
    try {
      const res = await fetch("/head/offsets");
      if (!res.ok) {
        showToast(`GET /head/offsets: ${res.status}`, true);
        return;
      }
      const o = (await res.json()) as HeadOffsets;
      setOffsets(o);
    } catch (e) {
      showToast(e instanceof Error ? e.message : String(e), true);
    }
  };

  onMount(() => {
    void loadSettings();
    void loadOffsets();
  });

  const saveTracker = async () => {
    try {
      // Re-fetch immediately before PUT so we merge against the
      // latest persisted state, not a snapshot captured at mount.
      // Without this, a Settings save mid-session would clobber the
      // tracker change we're about to make — and vice versa.
      const freshRes = await fetch("/settings");
      if (!freshRes.ok) {
        if (freshRes.status === 503) {
          throw new Error("settings unavailable (no SD card)");
        }
        throw new Error(`GET /settings: ${freshRes.status}`);
      }
      const fresh = (await freshRes.json()) as SettingsType;
      const body: SettingsType = { ...fresh, tracker: tracker() };
      const res = await authedFetch("/settings", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      if (!res.ok) {
        const text = await res.text().catch(() => "");
        throw new Error(`${res.status}: ${text || res.statusText}`);
      }
      const reply = (await res.json().catch(() => ({}))) as { reboot_required?: boolean };
      showToast(reply.reboot_required ? "tracker saved — reboot to apply" : "tracker saved");
    } catch (e) {
      showToast(e instanceof Error ? e.message : String(e), true);
    }
  };

  const saveOffsets = async () => {
    try {
      await postJson("/head/offsets", offsets());
      showToast(`offsets saved — yaw ${offsets().yaw_offset_deg}°, tilt ${offsets().tilt_offset_deg}°`);
    } catch (e) {
      showToast(e instanceof Error ? e.message : String(e), true);
    }
  };

  const capture = async () => {
    try {
      const captureRes = await authedFetch("/camera/capture", { method: "POST" });
      if (!captureRes.ok) {
        const text = await captureRes.text().catch(() => "");
        throw new Error(`${captureRes.status}: ${text || captureRes.statusText}`);
      }
      // The capture returns 202 immediately; the SD write completes
      // asynchronously. Wait long enough for the SPI burst to
      // finish before fetching, otherwise we re-render the prior
      // frame — confusing during calibration where the operator
      // expects each "Capture" click to reflect the current camera.
      await new Promise((r) => setTimeout(r, 350));
      const viewRes = await authedFetch("/camera/snapshot");
      if (!viewRes.ok) {
        const text = await viewRes.text().catch(() => "");
        throw new Error(`${viewRes.status}: ${text || viewRes.statusText}`);
      }
      const buf = new Uint8Array(await viewRes.arrayBuffer());
      const expected = FRAME_WIDTH * FRAME_HEIGHT * 2;
      if (buf.length !== expected) {
        throw new Error(`unexpected size ${buf.length}; want ${expected}`);
      }
      const ctx = canvasRef?.getContext("2d");
      if (!ctx) throw new Error("canvas 2d context unavailable");
      ctx.putImageData(rgb565beToImageData(buf), 0, 0);
      setHasSnapshot(true);
    } catch (e) {
      showToast(e instanceof Error ? e.message : String(e), true);
    }
  };

  const toggleAutoCapture = () => {
    const next = !autoCapture();
    setAutoCapture(next);
    if (next) {
      captureTimer = setInterval(() => void capture(), AUTO_CAPTURE_INTERVAL_MS);
      void capture(); // immediate first frame
    } else if (captureTimer != null) {
      clearInterval(captureTimer);
      captureTimer = null;
    }
  };

  onCleanup(() => {
    if (captureTimer != null) clearInterval(captureTimer);
  });

  const cameraMode = createMemo(() => snapshot()?.camera_mode ?? false);
  const toggleCameraMode = async () => {
    try {
      await postJson("/camera/mode", { enabled: !cameraMode() });
    } catch (e) {
      showToast(e instanceof Error ? e.message : String(e), true);
    }
  };

  return (
    <section>
      <h2>Calibration</h2>

      <h3>Camera</h3>
      <div class="btn-row">
        <button type="button" onClick={() => void capture()}>
          Capture &amp; view
        </button>
        <button type="button" onClick={toggleAutoCapture}>
          <Show when={autoCapture()} fallback="Auto-capture (every 2 s)">
            Stop auto-capture
          </Show>
        </button>
        <button type="button" onClick={() => void toggleCameraMode()}>
          <Show when={cameraMode()} fallback="LCD: avatar → camera">
            LCD: camera → avatar
          </Show>
        </button>
      </div>
      <canvas
        ref={canvasRef}
        width={FRAME_WIDTH}
        height={FRAME_HEIGHT}
        style={{
          display: hasSnapshot() ? "block" : "none",
          "margin-top": "8px",
          "max-width": "100%",
          border: "1px solid var(--muted)",
        }}
      />

      <h3>Tracker</h3>
      <div class="grid grid-2">
        <label>
          Horizontal FOV: <span>{tracker().fov_h_deg.toFixed(1)}°</span>
          <input
            type="range"
            min={20}
            max={120}
            step={0.5}
            value={tracker().fov_h_deg}
            onInput={(e) =>
              setTracker({ ...tracker(), fov_h_deg: Number(e.currentTarget.value) })
            }
          />
        </label>
        <label>
          Vertical FOV: <span>{tracker().fov_v_deg.toFixed(1)}°</span>
          <input
            type="range"
            min={20}
            max={120}
            step={0.5}
            value={tracker().fov_v_deg}
            onInput={(e) =>
              setTracker({ ...tracker(), fov_v_deg: Number(e.currentTarget.value) })
            }
          />
        </label>
        <label>
          Smoothing α: <span>{tracker().target_smoothing_alpha.toFixed(2)}</span>
          <input
            type="range"
            min={0.1}
            max={1.0}
            step={0.05}
            value={tracker().target_smoothing_alpha}
            onInput={(e) =>
              setTracker({
                ...tracker(),
                target_smoothing_alpha: Number(e.currentTarget.value),
              })
            }
          />
        </label>
        <div>
          <label>
            <input
              type="checkbox"
              checked={tracker().flip_x}
              onChange={(e) => setTracker({ ...tracker(), flip_x: e.currentTarget.checked })}
            />
            Flip X
          </label>
          <label style="margin-left:1em">
            <input
              type="checkbox"
              checked={tracker().flip_y}
              onChange={(e) => setTracker({ ...tracker(), flip_y: e.currentTarget.checked })}
            />
            Flip Y
          </label>
        </div>
      </div>
      <div class="btn-row">
        <button type="button" onClick={() => void saveTracker()}>
          Save tracker
        </button>
        <button type="button" onClick={() => void loadSettings()}>
          Reload
        </button>
        <button type="button" onClick={() => setTracker(TRACKER_DEFAULT)} title="Revert local edits to firmware defaults (does not persist; click Save to apply).">
          Reset to defaults
        </button>
      </div>

      <h3>Head offsets</h3>
      <div class="grid grid-2">
        <label>
          Yaw offset: <span>{offsets().yaw_offset_deg.toFixed(1)}°</span>
          <input
            type="range"
            min={-30}
            max={30}
            step={0.5}
            value={offsets().yaw_offset_deg}
            onInput={(e) =>
              setOffsets({ ...offsets(), yaw_offset_deg: Number(e.currentTarget.value) })
            }
          />
        </label>
        <label>
          Tilt offset: <span>{offsets().tilt_offset_deg.toFixed(1)}°</span>
          <input
            type="range"
            min={-30}
            max={30}
            step={0.5}
            value={offsets().tilt_offset_deg}
            onInput={(e) =>
              setOffsets({ ...offsets(), tilt_offset_deg: Number(e.currentTarget.value) })
            }
          />
        </label>
      </div>
      <div class="btn-row">
        <button type="button" onClick={() => void saveOffsets()}>
          Save offsets
        </button>
        <button type="button" onClick={() => void loadOffsets()}>
          Reload
        </button>
        <button type="button" onClick={() => setOffsets(HEAD_OFFSETS_DEFAULT)} title="Revert local edits to zero (does not persist; click Save to apply).">
          Reset to zero
        </button>
      </div>

      <small>
        FOV: place a target at a known angle, POST /look-at, see if the head over- or
        under-rotates; scale the FOV proportionally. Flips: move the target left of centre and
        check the head turns left too — if it goes the wrong way, toggle Flip X. Same for
        vertical with Flip Y. Smoothing α: 1.0 is pass-through (most responsive); lower values
        add EMA inertia. Head offsets: command pan = 0 / tilt = 0, eyeball whether the head is
        pointing dead ahead, then nudge the offsets so it does.
      </small>
    </section>
  );
}
