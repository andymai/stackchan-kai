import { For } from "solid-js";
import { postJson } from "../auth";
import { showToast } from "../store";

const EMOTIONS = ["neutral", "happy", "sad", "sleepy", "surprised", "angry"] as const;
const HOLD_MS = 30_000;

export function Emotion() {
  const send = async (emotion: string) => {
    try {
      await postJson("/emotion", { emotion, hold_ms: HOLD_MS });
      showToast(`emotion → ${emotion}`);
    } catch (e) {
      showToast((e as Error).message, true);
    }
  };

  const reset = async () => {
    try {
      await postJson("/reset", null);
      showToast("reset");
    } catch (e) {
      showToast((e as Error).message, true);
    }
  };

  return (
    <section>
      <h2>Emotion</h2>
      <div class="btn-row">
        <For each={EMOTIONS}>
          {(name) => (
            <button onClick={() => send(name)}>
              {name.charAt(0).toUpperCase() + name.slice(1)}
            </button>
          )}
        </For>
        <button onClick={reset} style="margin-left:auto">
          Reset
        </button>
      </div>
      <small>Holds the emotion for 30 s, then autonomous behaviour resumes.</small>
    </section>
  );
}
