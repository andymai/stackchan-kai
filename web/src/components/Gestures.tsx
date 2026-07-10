import { For } from "solid-js";
import { postJson } from "../auth";
import { showToast } from "../store";

// Wire-string vocabulary mirrors `NamedMotion::wire_str` in
// `crates/stackchan-core/src/motion.rs`.
const MOTIONS = ["greet", "nod", "shake", "laugh"] as const;

export function Gestures() {
  const send = async (motion: string) => {
    try {
      await postJson("/motion", { motion });
      showToast(`motion → ${motion}`);
    } catch (e) {
      showToast(e instanceof Error ? e.message : String(e), true);
    }
  };

  const power = async (path: "/sleep" | "/wake") => {
    try {
      await postJson(path, null);
      showToast(path === "/sleep" ? "sleeping" : "awake");
    } catch (e) {
      showToast(e instanceof Error ? e.message : String(e), true);
    }
  };

  return (
    <section>
      <h2>Gestures</h2>
      <div class="btn-row">
        <For each={MOTIONS}>
          {(name) => (
            <button onClick={() => void send(name)} data-motion={name}>
              {name.charAt(0).toUpperCase() + name.slice(1)}
            </button>
          )}
        </For>
      </div>
      <h3>Sleep</h3>
      <div class="btn-row">
        <button onClick={() => void power("/sleep")}>Sleep</button>
        <button onClick={() => void power("/wake")}>Wake</button>
      </div>
      <small>
        Gestures play a baked one-shot head + emotion script through the dance player. Sleep
        drops the eyes shut, head limp, LED ring dark, and audio TX paused; wake via the Wake
        button, any touch, or the power-key short-press. Runtime-only — sleep resets on reboot.
      </small>
    </section>
  );
}
