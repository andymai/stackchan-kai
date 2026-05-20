import { For, Show, createSignal, onCleanup, onMount } from "solid-js";
import type { EventEntry, EventsResponse } from "../types";
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

function kindClass(k: EventEntry["kind"]): string {
  return `tl-row tl-${k}`;
}

export function Events() {
  const [data, setData] = createSignal<EventsResponse | null>(null);
  // setTimeout self-reschedule instead of setInterval: a slow
  // `/events` fetch (firmware busy, network lag) would otherwise
  // queue a second tick on the prior interval boundary while the
  // first is still in flight, stacking N concurrent requests.
  let timer: ReturnType<typeof setTimeout> | null = null;
  let cancelled = false;

  const tick = async () => {
    try {
      const res = await fetch("/events");
      if (!res.ok) return;
      setData((await res.json()) as EventsResponse);
    } catch (e) {
      showToast(`/events: ${(e as Error).message}`, true);
    } finally {
      if (!cancelled) {
        timer = setTimeout(() => void tick(), POLL_MS);
      }
    }
  };

  onMount(() => {
    void tick();
  });
  onCleanup(() => {
    cancelled = true;
    if (timer != null) clearTimeout(timer);
  });

  return (
    <section>
      <h2>Events</h2>
      <Show
        when={data()}
        fallback={
          <div class="empty">
            <div class="empty-title">No events yet</div>
            <small>/events polls every 3 s.</small>
          </div>
        }
      >
        {(s) => (
          <>
            <div class="tl-head">
              <span class="tl-count">{s().total}</span>
              <span class="tl-count-label">since boot</span>
              <span class="tl-legend">
                <span class="tl-legend-item tl-lifecycle">lifecycle</span>
                <span class="tl-legend-item tl-control">control</span>
                <span class="tl-legend-item tl-warn">warn</span>
              </span>
            </div>
            <Show
              when={s().events.length > 0}
              fallback={<small>no events buffered</small>}
            >
              <ol class="timeline">
                <For each={s().events.slice().reverse()}>
                  {(e) => (
                    <li class={kindClass(e.kind)}>
                      <span class="tl-time">{fmtTimestamp(e.at_ms)}</span>
                      <span class="tl-dot" aria-hidden="true" />
                      <span class="tl-kind">{e.kind}</span>
                      <span class="tl-msg">{e.message}</span>
                    </li>
                  )}
                </For>
              </ol>
            </Show>
          </>
        )}
      </Show>
    </section>
  );
}
