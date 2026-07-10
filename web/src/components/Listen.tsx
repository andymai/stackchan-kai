import { Show } from "solid-js";
import { postJson } from "../auth";
import { showToast, snapshot } from "../store";

/// Default listen window in milliseconds — matches
/// `DEFAULT_LISTEN_DURATION_MS` in `stackchan-net::http_command`.
const DEFAULT_DURATION_MS = 3_000;

export function Listen() {
  const reply = () => snapshot()?.last_reply ?? null;

  const open = async () => {
    try {
      await postJson("/listen", { duration_ms: DEFAULT_DURATION_MS });
      showToast(`listening for ${DEFAULT_DURATION_MS / 1000}s`);
    } catch (e) {
      showToast((e as Error).message, true);
    }
  };

  return (
    <section>
      <h2>Listen</h2>
      <div class="btn-row">
        <button type="button" onClick={open}>
          Listen for 3s
        </button>
      </div>
      <Show when={reply()}>
        {(r) => (
          <div class={r().ok ? "listen-reply" : "listen-reply listen-reply-bad"}>
            <span class="listen-reply-label">{r().ok ? "reply" : "failed"}</span>
            <span class="listen-reply-text">{r().text}</span>
          </div>
        )}
      </Show>
      <small>
        Opens a 3-second listen window: arms the Ear decorator, sets Attention::Listening, and
        plays the wake-chirp. When an agent sidecar is configured, the reply lands here live via
        the state stream.
      </small>
    </section>
  );
}
