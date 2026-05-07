import { For } from "solid-js";
import { postJson } from "../auth";
import { showToast, snapshot } from "../store";

// Wire-string vocabulary mirrors `Mood::wire_str` in
// `crates/stackchan-core/src/mood.rs`.
const MOODS = ["neutral", "calm", "playful", "focus", "sleepy"] as const;

export function Mood() {
  const send = async (mood: string) => {
    try {
      await postJson("/mood", { mood });
      showToast(`mood → ${mood}`);
    } catch (e) {
      showToast((e as Error).message, true);
    }
  };

  return (
    <section>
      <h2>Mood</h2>
      <div class="btn-row">
        <For each={MOODS}>
          {(name) => {
            const active = () => snapshot()?.mood === name;
            return (
              <button
                onClick={() => send(name)}
                style={active() ? "border-color:var(--accent)" : ""}
                data-mood={name}
              >
                {name.charAt(0).toUpperCase() + name.slice(1)}
              </button>
            );
          }}
        </For>
      </div>
      <small>
        Mood scales blink rate, breath depth, and idle drift on top of the active emotion.
        Runtime-only — resets to Neutral on reboot.
      </small>
    </section>
  );
}
