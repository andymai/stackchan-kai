import { Show, createSignal, onCleanup } from "solid-js";
import { postJson } from "../auth";
import { showToast } from "../store";

const DEFAULT_DURATION_MS = 30_000;
const TICK_MS = 250;

export function Pair() {
  const [endsAt, setEndsAt] = createSignal<number | null>(null);
  const [now, setNow] = createSignal(Date.now());
  const [isPending, setIsPending] = createSignal(false);
  const remainingMs = () => {
    const e = endsAt();
    return e === null ? 0 : Math.max(0, e - now());
  };
  const isOpen = () => remainingMs() > 0;

  const timer = setInterval(() => {
    setNow(Date.now());
    if (endsAt() !== null && Date.now() >= (endsAt() ?? 0)) setEndsAt(null);
  }, TICK_MS);
  onCleanup(() => clearInterval(timer));

  const openWindow = async () => {
    if (isPending() || isOpen()) return;
    setIsPending(true);
    try {
      await postJson("/pair", { duration_ms: DEFAULT_DURATION_MS });
      setEndsAt(Date.now() + DEFAULT_DURATION_MS);
      showToast(`pairing window open for ${DEFAULT_DURATION_MS / 1000}s`);
    } catch (e) {
      showToast((e as Error).message, true);
    } finally {
      setIsPending(false);
    }
  };

  return (
    <section>
      <h2>Pair</h2>
      <div class="btn-row">
        <button
          type="button"
          onClick={openWindow}
          disabled={isPending() || isOpen()}
        >
          Open pairing window ({DEFAULT_DURATION_MS / 1000}s)
        </button>
        <Show when={isOpen()}>
          <span>{Math.ceil(remainingMs() / 1000)}s remaining</span>
        </Show>
      </div>
      <small>
        While open, the avatar shows the pairing decorator and accepts new
        ESP-NOW peer registrations.
      </small>
    </section>
  );
}
