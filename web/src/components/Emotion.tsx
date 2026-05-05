import { For, createSignal } from "solid-js";
import { postJson } from "../auth";
import { showToast } from "../store";

const EMOTIONS = ["neutral", "happy", "sad", "sleepy", "surprised", "angry"] as const;
const HOLD_PRESETS_S = [5, 10, 30, 60, 120] as const;

export function Emotion() {
  const [holdSec, setHoldSec] = createSignal<number>(30);

  const send = async (emotion: string) => {
    try {
      await postJson("/emotion", { emotion, hold_ms: holdSec() * 1000 });
      showToast(`emotion → ${emotion} (${holdSec()}s)`);
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
            <button onClick={() => send(name)} data-emotion={name}>
              {name.charAt(0).toUpperCase() + name.slice(1)}
            </button>
          )}
        </For>
        <button onClick={reset} style="margin-left:auto" data-shortcut="reset">
          Reset
        </button>
      </div>
      <label style="margin-top:8px">
        Hold: <span>{holdSec()} s</span>
        <div class="btn-row" style="margin-top:4px">
          <For each={HOLD_PRESETS_S}>
            {(v) => (
              <button
                type="button"
                onClick={() => setHoldSec(v)}
                style={holdSec() === v ? "border-color:var(--accent)" : ""}
              >
                {v}s
              </button>
            )}
          </For>
        </div>
      </label>
      <small>Keys 1-6 fire the emotion at the configured hold; R resets, M toggles mute.</small>
    </section>
  );
}
