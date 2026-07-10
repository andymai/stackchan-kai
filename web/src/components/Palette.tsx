import { For, createSignal } from "solid-js";
import { postJson } from "../auth";
import { showToast } from "../store";

// Wire-string vocabulary mirrors `Palette::wire_str` in
// `crates/stackchan-core/src/palette.rs`.
const PALETTES = ["default", "dark", "cute", "dog"] as const;

export function Palette() {
  // The avatar snapshot doesn't carry the active palette, so highlight
  // the last selection sent from this tab only.
  const [selected, setSelected] = createSignal<string | null>(null);

  const send = async (palette: string) => {
    try {
      await postJson("/palette", { palette });
      setSelected(palette);
      showToast(`palette → ${palette}`);
    } catch (e) {
      showToast(e instanceof Error ? e.message : String(e), true);
    }
  };

  return (
    <section>
      <h2>Palette</h2>
      <div class="btn-row">
        <For each={PALETTES}>
          {(name) => (
            <button
              onClick={() => void send(name)}
              style={selected() === name ? "border-color:var(--accent)" : ""}
              data-palette={name}
            >
              {name.charAt(0).toUpperCase() + name.slice(1)}
            </button>
          )}
        </For>
      </div>
      <small>
        Swaps the avatar's skin / eye / mouth colours at runtime. Persists across reboots via the
        runtime store and wins over the System page's boot appearance pin.
      </small>
    </section>
  );
}
