import { Show, createSignal, onMount } from "solid-js";
import { authedFetch } from "../auth";
import { showToast } from "../store";

// Top-of-dashboard banner that surfaces the firmware's most recent
// panic log. Backed by the persistent crash latch in RTC fast RAM
// (firmware/src/crash.rs) → /sd/CRASH.LOG → GET /crash.
//
// Lifecycle:
// - Mount: GET /crash. 200 → render banner with the log body.
//   404/503 → silent (no recent crash, or no SD).
// - Operator clicks Dismiss → POST /crash/clear, then 204 → hide.
export function CrashBanner() {
  const [body, setBody] = createSignal<string | null>(null);

  const load = async () => {
    try {
      const res = await fetch("/crash");
      if (res.status === 200) {
        const text = await res.text();
        setBody(text);
      } else {
        setBody(null);
      }
    } catch {
      // Best-effort surface; absent /crash isn't a UI error.
      setBody(null);
    }
  };

  const dismiss = async () => {
    try {
      const res = await authedFetch("/crash/clear", { method: "POST" });
      if (!res.ok) {
        const text = await res.text().catch(() => "");
        throw new Error(`${res.status}: ${text || res.statusText}`);
      }
      setBody(null);
      showToast("crash log cleared");
    } catch (e) {
      showToast(e instanceof Error ? e.message : String(e), true);
    }
  };

  onMount(() => void load());

  return (
    <Show when={body()}>
      <section
        style={{
          "border-left": "4px solid #d33",
          "background-color": "rgba(221, 51, 51, 0.08)",
          padding: "8px 12px",
          "margin-bottom": "12px",
        }}
      >
        <div style={{ display: "flex", "justify-content": "space-between", gap: "8px" }}>
          <strong>Previous boot crashed</strong>
          <button type="button" onClick={() => void dismiss()}>
            Dismiss
          </button>
        </div>
        <pre
          style={{
            "white-space": "pre-wrap",
            "font-size": "0.85em",
            "margin-top": "6px",
            "margin-bottom": "0",
          }}
        >
          {body()}
        </pre>
      </section>
    </Show>
  );
}
