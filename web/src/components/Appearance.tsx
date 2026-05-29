import { createSignal, onMount } from "solid-js";
import { authedFetch } from "../auth";
import { showToast } from "../store";
import type { Settings as SettingsType } from "../types";

// Wire-string vocabularies mirror `Palette::wire_str` and
// `FaceGeometry::wire_str` in `crates/stackchan-core/src/{palette,face}.rs`.
// The empty string is the "(not pinned)" sentinel the firmware reads as
// "fall back to the variant default".
const PALETTES = ["", "default", "dark", "cute", "dog"] as const;
const GEOMETRIES = ["", "default", "chibi", "wide", "sleepy"] as const;

const label = (v: string) => (v === "" ? "(not pinned)" : v);

export function Appearance() {
  const [palette, setPalette] = createSignal("");
  const [geometry, setGeometry] = createSignal("");

  const load = async () => {
    try {
      const res = await fetch("/settings");
      if (!res.ok) {
        if (res.status === 503) {
          showToast("settings unavailable (no SD card)", true);
        } else {
          showToast(`GET /settings: ${res.status}`, true);
        }
        return;
      }
      const c = (await res.json()) as SettingsType;
      setPalette(c.appearance.palette);
      setGeometry(c.appearance.face_geometry);
    } catch (e) {
      showToast(e instanceof Error ? e.message : String(e), true);
    }
  };

  onMount(load);

  const submit = async (ev: Event) => {
    ev.preventDefault();
    try {
      // Re-fetch before PUT and overwrite only the appearance block, so a
      // concurrent save through the Settings form (or a hand-edit) isn't
      // clobbered — same merge-against-fresh discipline as Settings.tsx.
      const freshRes = await fetch("/settings");
      if (!freshRes.ok) {
        if (freshRes.status === 503) {
          throw new Error("settings unavailable (no SD card)");
        }
        throw new Error(`GET /settings: ${freshRes.status}`);
      }
      const fresh = (await freshRes.json()) as SettingsType;
      const body: SettingsType = {
        ...fresh,
        appearance: { palette: palette(), face_geometry: geometry() },
      };
      const res = await authedFetch("/settings", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      if (!res.ok) {
        const text = await res.text().catch(() => "");
        throw new Error(`${res.status}: ${text || res.statusText}`);
      }
      showToast("saved — boot appearance applies on next reboot");
    } catch (e) {
      showToast(e instanceof Error ? e.message : String(e), true);
    }
  };

  return (
    <section>
      <h2>Boot appearance</h2>
      <form class="grid" onSubmit={submit}>
        <label>
          Palette
          <select value={palette()} onChange={(e) => setPalette(e.currentTarget.value)}>
            {PALETTES.map((p) => (
              <option value={p}>{label(p)}</option>
            ))}
          </select>
        </label>
        <label>
          Face geometry
          <select value={geometry()} onChange={(e) => setGeometry(e.currentTarget.value)}>
            {GEOMETRIES.map((g) => (
              <option value={g}>{label(g)}</option>
            ))}
          </select>
        </label>
        <div class="btn-row">
          <button type="submit">Save</button>
          <button type="button" onClick={load}>
            Reload
          </button>
        </div>
        <small>
          These are BOOT defaults: they seed the runtime store only on first
          boot (when <code>RUNTIME.RON</code> is absent). A later live change via
          the Behavior page (POST /palette / POST /face-geometry) persists to
          the runtime store and wins on subsequent boots. Leave a field as
          <code>(not pinned)</code> to fall back to the firmware default.
        </small>
      </form>
    </section>
  );
}
