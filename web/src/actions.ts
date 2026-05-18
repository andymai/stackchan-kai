import { postJson } from "./auth";
import { showToast, snapshot } from "./store";

export async function resetEmotion(): Promise<void> {
  try {
    await postJson("/reset", null);
    showToast("reset");
  } catch (e) {
    showToast((e as Error).message, true);
  }
}

export async function toggleMute(): Promise<void> {
  const next = !(snapshot()?.audio.muted ?? false);
  try {
    await postJson("/mute", { muted: next });
    showToast(next ? "muted" : "unmuted");
  } catch (e) {
    showToast((e as Error).message, true);
  }
}
