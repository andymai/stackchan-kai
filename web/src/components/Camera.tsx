import { Show, createMemo } from "solid-js";
import { authedFetch, postJson } from "../auth";
import { showToast, snapshot } from "../store";

export function Camera() {
  const previewActive = createMemo(() => snapshot()?.camera_mode ?? false);

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
      showToast("capture queued — eject SD to view CAPTURE.565");
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
      </div>
      <small>Preview swaps the LCD between avatar and live camera; tracking continues either way. Capture writes /sd/CAPTURE.565 (raw QVGA RGB565, ~150 KB).</small>
    </section>
  );
}
