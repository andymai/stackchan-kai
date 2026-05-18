import { For } from "solid-js";
import { SECTIONS, section, goto } from "../nav";
import { conn, snapshot } from "../store";

export function Sidebar(props: { onNavigate?: () => void }) {
  return (
    <aside class="sidebar">
      <div class="brand">
        <div class="brand-mark">SC</div>
        <div class="brand-text">
          <div class="brand-name">STACK-CHAN</div>
          <div class="brand-sub">operator console</div>
        </div>
      </div>

      <nav class="nav" aria-label="Sections">
        <For each={SECTIONS}>
          {(s) => (
            <button
              type="button"
              class="nav-item"
              classList={{ active: section() === s.id }}
              aria-current={section() === s.id ? "page" : undefined}
              aria-label={`${s.label} (press g then ${s.hotkey})`}
              onClick={() => {
                goto(s.id);
                props.onNavigate?.();
              }}
            >
              <span class="nav-glyph" aria-hidden="true">
                {s.glyph}
              </span>
              <span class="nav-label">{s.label}</span>
              <span class="nav-key" aria-hidden="true">
                g {s.hotkey}
              </span>
            </button>
          )}
        </For>
      </nav>

      <div class="sidebar-foot">
        <div
          class="link-status"
          classList={{ ok: conn() === "ok", bad: conn() === "bad" }}
          role="status"
          aria-live="polite"
        >
          <span class="link-dot" aria-hidden="true" />
          <span class="link-label">
            {conn() === "ok" ? "LINK UP" : conn() === "bad" ? "LINK DOWN" : "LINKING"}
          </span>
        </div>
        <div class="link-host" aria-label="Device address">
          {snapshot()?.wifi.ip ?? "stackchan.local"}
        </div>
        <div class="link-hint">
          Press <kbd>?</kbd> for shortcuts
        </div>
      </div>
    </aside>
  );
}
