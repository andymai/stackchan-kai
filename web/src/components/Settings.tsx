import { createSignal, onMount } from "solid-js";
import { putJson, setAuthToken } from "../auth";
import { showToast, snapshot } from "../store";
import type { Settings as SettingsType } from "../types";

export function Settings() {
  const [ssid, setSsid] = createSignal("");
  const [psk, setPsk] = createSignal("");
  const [country, setCountry] = createSignal("US");
  const [hostname, setHostname] = createSignal("stackchan");
  const [sntp, setSntp] = createSignal("");
  const [token, setToken] = createSignal("");
  const [tz, setTz] = createSignal("UTC");

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
      setSsid(c.wifi.ssid);
      setPsk(c.wifi.psk);
      setCountry(c.wifi.country);
      setHostname(c.mdns.hostname);
      setSntp(c.time.sntp_servers.join(", "));
      setToken(c.auth.token);
      setTz(c.time.tz);
    } catch (e) {
      showToast((e as Error).message, true);
    }
  };

  onMount(load);

  const submit = async (ev: Event) => {
    ev.preventDefault();
    const audio = snapshot()?.audio ?? { volume_pct: 50, muted: false };
    const body: SettingsType = {
      wifi: { ssid: ssid(), psk: psk(), country: country().toUpperCase() },
      mdns: { hostname: hostname() },
      time: {
        tz: tz(),
        sntp_servers: sntp()
          .split(",")
          .map((s) => s.trim())
          .filter(Boolean),
      },
      auth: { token: token() },
      audio,
    };
    const newToken = body.auth.token;
    try {
      await putJson("/settings", body);
      setAuthToken(newToken);
      showToast("settings saved — reboot to apply");
    } catch (e) {
      showToast((e as Error).message, true);
    }
  };

  return (
    <section>
      <h2>Settings</h2>
      <form class="grid" onSubmit={submit}>
        <label>
          SSID
          <input
            type="text"
            autocomplete="off"
            value={ssid()}
            onInput={(e) => setSsid(e.currentTarget.value)}
          />
        </label>
        <label>
          PSK
          <input
            type="password"
            autocomplete="off"
            placeholder="(cleared = open AP)"
            value={psk()}
            onInput={(e) => setPsk(e.currentTarget.value)}
          />
        </label>
        <label>
          Country
          <input
            type="text"
            maxlength={2}
            value={country()}
            onInput={(e) => setCountry(e.currentTarget.value)}
          />
        </label>
        <label>
          mDNS hostname
          <input
            type="text"
            value={hostname()}
            onInput={(e) => setHostname(e.currentTarget.value)}
          />
        </label>
        <label>
          SNTP servers (comma-separated)
          <input
            type="text"
            value={sntp()}
            onInput={(e) => setSntp(e.currentTarget.value)}
          />
        </label>
        <label>
          Auth token
          <input
            type="password"
            autocomplete="off"
            value={token()}
            onInput={(e) => setToken(e.currentTarget.value)}
          />
        </label>
        <div class="btn-row">
          <button type="submit">Save (reboot to apply)</button>
          <button type="button" onClick={load}>
            Reload
          </button>
        </div>
        <small>
          PSK and token show as <code>***</code> in GET and pre-fill into the form.
          Submit unchanged to keep the current value, clear the field to disable
          (open AP / auth off), or type a new value to overwrite.
        </small>
      </form>
    </section>
  );
}
