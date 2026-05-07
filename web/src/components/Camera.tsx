import { Show, createMemo, createSignal } from "solid-js";
import { authedFetch, postJson } from "../auth";
import { showToast, snapshot } from "../store";

/// QVGA frame width — must match the firmware tracker's `WIDTH`.
const FRAME_WIDTH = 320;
/// QVGA frame height — must match the firmware tracker's `HEIGHT`.
const FRAME_HEIGHT = 240;

/// Convert a raw QVGA RGB565 big-endian frame into an `ImageData`
/// the canvas can blit. Each input pixel is 2 bytes (BE):
///
/// - byte 0: `RRRRR_GGG`
/// - byte 1: `GGG_BBBBB`
function rgb565beToImageData(buf: Uint8Array): ImageData {
  const out = new ImageData(FRAME_WIDTH, FRAME_HEIGHT);
  const dst = out.data;
  // DataView gives type-safe big-endian reads without the `noUncheckedIndexedAccess`
  // narrowing that makes plain `buf[i]` come back as `number | undefined`.
  const view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
  for (let i = 0; i < FRAME_WIDTH * FRAME_HEIGHT; i++) {
    const px = view.getUint16(i * 2, false);
    // Expand 5/6/5 to 8 bits per channel by replicating high bits into
    // the low bits — common upscale that avoids the dimming you'd
    // get from a plain left-shift.
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

export function Camera() {
  const previewActive = createMemo(() => snapshot()?.camera_mode ?? false);
  // Visibility marker for the canvas — `true` once a snapshot has
  // been fetched and rendered. Plain bool because we never hold the
  // image bytes in JS state; the canvas owns them.
  const [hasSnapshot, setHasSnapshot] = createSignal(false);
  let canvasRef: HTMLCanvasElement | undefined;

  const toggleMode = async () => {
    const next = !previewActive();
    try {
      await postJson("/camera/mode", { enabled: next });
      showToast(next ? "camera preview" : "avatar mode");
    } catch (e) {
      showToast((e as Error).message, true);
    }
  };

  const capture = async () => {
    try {
      const res = await authedFetch("/camera/capture", { method: "POST" });
      if (!res.ok) {
        const text = await res.text().catch(() => "");
        throw new Error(`${res.status}: ${text || res.statusText}`);
      }
      showToast("capture queued — fetch with View capture in ~500 ms");
    } catch (e) {
      showToast((e as Error).message, true);
    }
  };

  const view = async () => {
    try {
      const res = await authedFetch("/camera/snapshot");
      if (!res.ok) {
        const text = await res.text().catch(() => "");
        throw new Error(`${res.status}: ${text || res.statusText}`);
      }
      const buf = new Uint8Array(await res.arrayBuffer());
      const expected = FRAME_WIDTH * FRAME_HEIGHT * 2;
      if (buf.length !== expected) {
        throw new Error(`unexpected size ${buf.length}; want ${expected}`);
      }
      const ctx = canvasRef?.getContext("2d");
      if (!ctx) throw new Error("canvas 2d context unavailable");
      ctx.putImageData(rgb565beToImageData(buf), 0, 0);
      setHasSnapshot(true);
      showToast("snapshot rendered");
    } catch (e) {
      showToast((e as Error).message, true);
    }
  };

  return (
    <section>
      <h2>Camera</h2>
      <div class="btn-row">
        <button type="button" onClick={toggleMode}>
          <Show when={previewActive()} fallback="Show preview">
            Hide preview
          </Show>
        </button>
        <button type="button" onClick={capture}>
          Capture frame
        </button>
        <button type="button" onClick={view}>
          View capture
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
      <small>
        Preview swaps the LCD between avatar and live camera; tracking continues either way.
        Capture writes /sd/CAPTURE.565 (raw QVGA RGB565); View capture fetches and renders it.
      </small>
    </section>
  );
}
