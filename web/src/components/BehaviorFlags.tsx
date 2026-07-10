import { For, Show, createSignal, onMount } from "solid-js";
import { authedFetch } from "../auth";
import { showToast } from "../store";
import type { Settings as SettingsType } from "../types";

// Field vocabulary mirrors `BehaviorFlagUpdate` in
// `crates/stackchan-net/src/http_command.rs` — the four booleans
// POST /behavior accepts. Reboot-only fields (wake_word_*) stay
// behind PUT /settings on the System page.
const FLAGS = [
  { field: "soliloquy_enabled", label: "Soliloquy" },
  { field: "hourly_chime_enabled", label: "Hourly chime" },
  { field: "battery_icon_enabled", label: "Battery icon" },
  { field: "toast_overlay_enabled", label: "Toast overlay" },
] as const;

type FlagField = (typeof FLAGS)[number]["field"];
type FlagState = Record<FlagField, boolean>;

export function BehaviorFlags() {
  const [flags, setFlags] = createSignal<FlagState | null>(null);
  const [loadFailed, setLoadFailed] = createSignal(false);

  const load = async () => {
    setLoadFailed(false);
    try {
      const res = await fetch("/settings");
      if (!res.ok) {
        setLoadFailed(true);
        if (res.status === 503) {
          showToast("behavior flags unavailable (no SD card)", true);
        } else {
          showToast(`GET /settings: ${res.status}`, true);
        }
        return;
      }
      const c = (await res.json()) as SettingsType;
      setFlags({
        soliloquy_enabled: c.behavior.soliloquy_enabled,
        hourly_chime_enabled: c.behavior.hourly_chime_enabled,
        battery_icon_enabled: c.behavior.battery_icon_enabled,
        toast_overlay_enabled: c.behavior.toast_overlay_enabled,
      });
    } catch (e) {
      setLoadFailed(true);
      showToast(e instanceof Error ? e.message : String(e), true);
    }
  };

  onMount(load);

  const toggle = async (field: FlagField, value: boolean): Promise<boolean> => {
    try {
      const res = await authedFetch("/behavior", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ field, value }),
      });
      if (!res.ok) {
        const text = await res.text().catch(() => "");
        throw new Error(`${res.status}: ${text || res.statusText}`);
      }
      setFlags((curr) => (curr ? { ...curr, [field]: value } : curr));
      const reply = (await res.json().catch(() => ({}))) as { reboot_required?: boolean };
      showToast(
        reply.reboot_required
          ? `${field} → ${value ? "on" : "off"} — reboot to apply`
          : `${field} → ${value ? "on" : "off"}`,
      );
      return true;
    } catch (e) {
      showToast(e instanceof Error ? e.message : String(e), true);
      return false;
    }
  };

  return (
    <section>
      <h2>Flags</h2>
      <Show
        when={flags()}
        fallback={
          <Show when={loadFailed()} fallback={<small>Loading…</small>}>
            <small>
              Flags unavailable — GET /settings failed (no SD card?).{" "}
              <button type="button" onClick={() => void load()}>
                Retry
              </button>
            </small>
          </Show>
        }
      >
        {(f) => (
          <div class="btn-row">
            <For each={FLAGS}>
              {({ field, label }) => (
                <label>
                  <input
                    type="checkbox"
                    checked={f()[field]}
                    onChange={(e) => {
                      const el = e.currentTarget;
                      void toggle(field, el.checked).then((ok) => {
                        if (!ok) el.checked = f()[field];
                      });
                    }}
                  />
                  {label}
                </label>
              )}
            </For>
          </div>
        )}
      </Show>
      <small>
        Each toggle persists one boolean to <code>STACKCHAN.RON</code> via POST /behavior. The
        consuming task captures its flag at boot, so changes take effect on the next reboot —
        the firmware replies <code>reboot_required</code> to make that explicit.
      </small>
    </section>
  );
}
