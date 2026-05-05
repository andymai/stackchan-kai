import { authedFetch } from "../auth";
import { showToast } from "../store";

async function postRaw(path: string, body?: string): Promise<void> {
  const init: RequestInit = { method: "POST" };
  if (body != null) {
    init.body = body;
    init.headers = { "Content-Type": "application/json" };
  }
  const res = await authedFetch(path, init);
  if (!res.ok) {
    const text = await res.text().catch(() => "");
    throw new Error(`${res.status}: ${text || res.statusText}`);
  }
}

export function Recovery() {
  const restart = async () => {
    if (!confirm("Soft-reset the device? Active SSE connections will drop and reconnect.")) {
      return;
    }
    try {
      await postRaw("/restart");
      showToast("device rebooting…");
    } catch (e) {
      showToast((e as Error).message, true);
    }
  };

  const factoryReset = async () => {
    const phrase = prompt(
      'Factory reset wipes STACKCHAN.RON, BLE bonds, and the camera capture, then reboots.\n\nType "erase" to confirm.',
    );
    if (phrase !== "erase") {
      if (phrase != null) showToast("factory-reset cancelled", true);
      return;
    }
    try {
      await postRaw("/factory-reset", JSON.stringify({ confirm: "erase" }));
      showToast("device wiping + rebooting…");
    } catch (e) {
      showToast((e as Error).message, true);
    }
  };

  return (
    <section>
      <h2>Recovery</h2>
      <div class="btn-row">
        <button type="button" onClick={restart}>
          Restart
        </button>
        <button
          type="button"
          onClick={factoryReset}
          style="margin-left:auto;border-color:var(--bad);color:var(--bad)"
        >
          Factory reset…
        </button>
      </div>
      <small>
        Both routes always require an authenticated bearer token, even when
        global auth is disabled — destructive ops opt-in only. Restart soft-
        resets via esp_hal::system::software_reset; factory-reset additionally
        wipes STACKCHAN.RON, BONDS.BIN, CAPTURE.565, and the staging files.
      </small>
    </section>
  );
}
