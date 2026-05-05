import { createSignal } from "solid-js";
import { postJson } from "../auth";
import { showToast } from "../store";

const PHRASES = [
  "wake_chirp",
  "pickup_chirp",
  "startle_chirp",
  "low_battery_chirp",
  "camera_mode_entered_chirp",
  "camera_mode_exited_chirp",
  "greeting",
  "acknowledge_name",
  "battery_low",
] as const;

const LOCALES = ["en", "ja"] as const;

export function Speak() {
  const [phrase, setPhrase] = createSignal<(typeof PHRASES)[number]>("greeting");
  const [locale, setLocale] = createSignal<(typeof LOCALES)[number]>("en");

  const send = async () => {
    try {
      await postJson("/speak", { phrase: phrase(), locale: locale() });
      showToast(`speak ${phrase()} (${locale()})`);
    } catch (e) {
      showToast((e as Error).message, true);
    }
  };

  return (
    <section>
      <h2>Speak</h2>
      <div class="grid grid-2">
        <label>
          Phrase
          <select
            onChange={(e) => setPhrase(e.currentTarget.value as (typeof PHRASES)[number])}
          >
            {PHRASES.map((p) => (
              <option value={p} selected={p === phrase()}>
                {p}
              </option>
            ))}
          </select>
        </label>
        <label>
          Locale
          <select
            onChange={(e) => setLocale(e.currentTarget.value as (typeof LOCALES)[number])}
          >
            {LOCALES.map((l) => (
              <option value={l} selected={l === locale()}>
                {l}
              </option>
            ))}
          </select>
        </label>
      </div>
      <div class="btn-row" style="margin-top:8px">
        <button type="button" onClick={send}>
          Speak
        </button>
      </div>
      <small>Phrases bake into firmware as PCM clips. SFX chirps fall back to the same locale either way.</small>
    </section>
  );
}
