import { postJson } from "../auth";
import { showToast } from "../store";

/// Default listen window in milliseconds — matches
/// `DEFAULT_LISTEN_DURATION_MS` in `stackchan-net::http_command`.
const DEFAULT_DURATION_MS = 3_000;

export function Listen() {
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
      <small>
        Opens a 3-second listen window: arms the Ear decorator, sets Attention::Listening, and
        plays the wake-chirp. Wake-word detection ships in a follow-up — for now this is the
        push-to-talk equivalent for the avatar's listening pose.
      </small>
    </section>
  );
}
