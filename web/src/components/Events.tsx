import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import type { EventsResponse } from "../types";
import { showToast } from "../store";

const POLL_MS = 3000;

function fmtTimestamp(ms: number): string {
  const totalSec = Math.floor(ms / 1000);
  const h = Math.floor(totalSec / 3600);
  const m = Math.floor((totalSec % 3600) / 60);
  const s = totalSec % 60;
  if (h > 0) return `${h}h${m.toString().padStart(2, "0")}m${s.toString().padStart(2, "0")}s`;
  if (m > 0) return `${m}m${s.toString().padStart(2, "0")}s`;
  return `${s}s`;
}

export function Events() {
  const [data, setData] = createSignal<EventsResponse | null>(null);
  let timer: ReturnType<typeof setInterval> | null = null;

  const tick = async () => {
    try {
      const res = await fetch("/events");
      if (!res.ok) return;
      setData((await res.json()) as EventsResponse);
    } catch (e) {
      showToast(`/events: ${(e as Error).message}`, true);
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
      <h2>Events</h2>
      <Show when={data()} fallback={<small>waiting for first poll…</small>}>
        {(s) => (
          <>
            <small>{s().total} since boot</small>
            <div class="grid" style="margin-top:8px;font-family:ui-monospace,monospace;font-size:12px">
              <For each={s().events.slice().reverse()}>
                {(e) => (
                  <div style="display:grid;grid-template-columns:max-content max-content 1fr;gap:8px;align-items:baseline">
                    <span style="color:var(--muted);font-variant-numeric:tabular-nums">
                      {fmtTimestamp(e.at_ms)}
                    </span>
                    <span style={`color:${e.kind === "warn" ? "var(--bad)" : e.kind === "control" ? "var(--accent)" : "var(--muted)"}`}>
                      {e.kind}
                    </span>
                    <span>{e.message}</span>
                  </div>
                )}
              </For>
            </div>
          </>
        )}
      </Show>
    </section>
  );
}
