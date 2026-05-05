import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import type { TasksSnapshot } from "../types";
import { showToast } from "../store";

const POLL_MS = 5000;

export function TaskHealth() {
  const [data, setData] = createSignal<TasksSnapshot | null>(null);
  let timer: ReturnType<typeof setInterval> | null = null;

  const tick = async () => {
    try {
      const res = await fetch("/tasks");
      if (!res.ok) return;
      setData((await res.json()) as TasksSnapshot);
    } catch (e) {
      showToast(`/tasks: ${(e as Error).message}`, true);
    }
  };

  onMount(() => {
    void tick();
    timer = setInterval(tick, POLL_MS);
  });
  onCleanup(() => {
    if (timer != null) clearInterval(timer);
  });

  return (
    <section>
      <h2>Task health</h2>
      <Show when={data()} fallback={<small>waiting for first watchdog window…</small>}>
        {(s) => (
          <>
            <dl class="row">
              <For each={s().channels}>
                {(ch) => (
                  <>
                    <dt>{ch.name}</dt>
                    <dd>
                      <span class={`dot ${ch.stale ? "bad" : "ok"}`} style="display:inline-block;margin-right:6px" />
                      {ch.delta} / ≥{ch.min_per_window} beats
                    </dd>
                  </>
                )}
              </For>
            </dl>
            <small>Window: {s().window_ms} ms. Stale = below the cadence-derived minimum.</small>
          </>
        )}
      </Show>
    </section>
  );
}
