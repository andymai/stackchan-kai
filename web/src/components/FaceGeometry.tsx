import { For } from "solid-js";
import { postJson } from "../auth";
import { showToast, snapshot } from "../store";

// Wire-string vocabulary mirrors `FaceGeometry::wire_str` in
// `crates/stackchan-core/src/face.rs`.
const GEOMETRIES = ["default", "chibi", "wide", "sleepy"] as const;

export function FaceGeometry() {
  const send = async (geometry: string) => {
    try {
      await postJson("/face-geometry", { geometry });
      showToast(`face → ${geometry}`);
    } catch (e) {
      showToast((e as Error).message, true);
    }
  };

  return (
    <section>
      <h2>Face</h2>
      <div class="btn-row">
        <For each={GEOMETRIES}>
          {(name) => {
            const active = () => snapshot()?.face_geometry === name;
            return (
              <button
                onClick={() => send(name)}
                style={active() ? "border-color:var(--accent)" : ""}
                data-face-geometry={name}
              >
                {name.charAt(0).toUpperCase() + name.slice(1)}
              </button>
            );
          }}
        </For>
      </div>
      <small>
        Picks the eye + mouth baseline silhouette. Emotion still scales eye size, blink rate, and
        breath depth on top of this. Persists across reboots via the runtime store.
      </small>
    </section>
  );
}
