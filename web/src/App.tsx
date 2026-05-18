import { Match, Show, Switch, createSignal, onCleanup, onMount } from "solid-js";
import { connectStream } from "./store";
import { resetEmotion, toggleMute } from "./actions";
import { SECTIONS, goto, section, useHashRouter } from "./nav";
import { Sidebar } from "./components/Sidebar";
import { StatusBar } from "./components/StatusBar";
import { CrashBanner } from "./components/CrashBanner";
import { State } from "./components/State";
import { Emotion } from "./components/Emotion";
import { Listen } from "./components/Listen";
import { Mood } from "./components/Mood";
import { FaceGeometry } from "./components/FaceGeometry";
import { LookAt } from "./components/LookAt";
import { Audio } from "./components/Audio";
import { Camera } from "./components/Camera";
import { Calibration } from "./components/Calibration";
import { Events } from "./components/Events";
import { Recovery } from "./components/Recovery";
import { Sensors } from "./components/Sensors";
import { Settings } from "./components/Settings";
import { Pair } from "./components/Pair";
import { Speak } from "./components/Speak";
import { TaskHealth } from "./components/TaskHealth";
import { Telemetry } from "./components/Telemetry";
import { Toast } from "./components/Toast";

const EMOTION_KEYS: Record<string, string> = {
  "1": "neutral",
  "2": "happy",
  "3": "sad",
  "4": "sleepy",
  "5": "surprised",
  "6": "angry",
  "7": "doubt",
  "8": "boring",
  "9": "hi",
  "0": "loved",
  q: "curious",
  w: "confused",
  e: "mad",
};

const NAV_KEYS = Object.fromEntries(SECTIONS.map((s) => [s.hotkey, s.id]));

function PageHead(props: { title: string; tag?: string }) {
  return (
    <div class="page-head">
      <h2 class="page-title" aria-live="polite">{props.title}</h2>
      <div class="page-rule" aria-hidden="true" />
      <Show when={props.tag}>{(t) => <span class="page-tag">{t()}</span>}</Show>
    </div>
  );
}

function KbdOverlay(props: { onClose: () => void }) {
  let panel: HTMLDivElement | undefined;
  const prevFocus = document.activeElement as HTMLElement | null;

  onMount(() => panel?.focus());
  onCleanup(() => prevFocus?.focus?.());

  return (
    <div class="kbd-overlay" onClick={props.onClose} role="presentation">
      <div
        ref={panel}
        class="kbd-panel"
        role="dialog"
        aria-modal="true"
        aria-label="Keyboard shortcuts"
        tabindex={-1}
        onClick={(e) => e.stopPropagation()}
      >
        <h3>Shortcuts</h3>
        <dl class="kbd-grid">
          <dt>1-9 0 q w e</dt>
          <dd>set emotion (Behavior)</dd>
          <dt>r</dt>
          <dd>reset</dd>
          <dt>m</dt>
          <dd>toggle mute</dd>
          <dt>g s/b/m/v/y/d/c</dt>
          <dd>go to section</dd>
          <dt>?</dt>
          <dd>this overlay</dd>
          <dt>Esc</dt>
          <dd>close</dd>
        </dl>
      </div>
    </div>
  );
}

export function App() {
  const [overlay, setOverlay] = createSignal(false);
  let awaitingG = false;
  let gTimer: ReturnType<typeof setTimeout> | null = null;

  const clearG = () => {
    awaitingG = false;
    if (gTimer != null) {
      clearTimeout(gTimer);
      gTimer = null;
    }
  };

  const onKeyDown = (ev: KeyboardEvent) => {
    const t = ev.target as HTMLElement | null;
    if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT")) {
      return;
    }
    if (ev.altKey || ev.ctrlKey || ev.metaKey) return;

    const k = ev.key.toLowerCase();

    if (ev.key === "Escape") {
      if (overlay()) {
        setOverlay(false);
        ev.preventDefault();
      }
      clearG();
      return;
    }
    if (ev.key === "?") {
      setOverlay((v) => !v);
      ev.preventDefault();
      clearG();
      return;
    }
    if (overlay()) return;

    if (awaitingG) {
      clearG();
      const target = NAV_KEYS[k];
      if (target) {
        goto(target as (typeof SECTIONS)[number]["id"]);
        ev.preventDefault();
      }
      return;
    }

    if (k === "g") {
      awaitingG = true;
      gTimer = setTimeout(clearG, 900);
      ev.preventDefault();
      return;
    }

    const emotion = EMOTION_KEYS[k];
    if (emotion) {
      document.querySelector<HTMLButtonElement>(`button[data-emotion="${emotion}"]`)?.click();
      ev.preventDefault();
      return;
    }
    if (k === "r") {
      void resetEmotion();
      ev.preventDefault();
      return;
    }
    if (k === "m") {
      void toggleMute();
      ev.preventDefault();
    }
  };

  onMount(() => {
    connectStream();
    document.addEventListener("keydown", onKeyDown);
  });
  onCleanup(() => document.removeEventListener("keydown", onKeyDown));
  useHashRouter();

  return (
    <>
      <div class="shell">
        <Sidebar />
        <main class="main" aria-label="Operator console">
          <StatusBar />
          <CrashBanner />
          <div class="content">
            <Switch>
              <Match when={section() === "status"}>
                <PageHead title="Status" tag="LIVE" />
                <State />
                <Telemetry />
              </Match>
              <Match when={section() === "behavior"}>
                <PageHead title="Behavior" />
                <Emotion />
                <Mood />
                <FaceGeometry />
              </Match>
              <Match when={section() === "motion"}>
                <PageHead title="Motion" />
                <LookAt />
                <Calibration />
              </Match>
              <Match when={section() === "voice"}>
                <PageHead title="Voice" />
                <Audio />
                <Speak />
                <Listen />
              </Match>
              <Match when={section() === "vision"}>
                <PageHead title="Vision" />
                <Camera />
                <Pair />
              </Match>
              <Match when={section() === "diagnostics"}>
                <PageHead title="Diagnostics" />
                <TaskHealth />
                <Sensors />
                <Events />
              </Match>
              <Match when={section() === "system"}>
                <PageHead title="System" />
                <Settings />
                <Recovery />
              </Match>
            </Switch>
          </div>
        </main>
      </div>
      <Toast />
      <Show when={overlay()}>
        <KbdOverlay onClose={() => setOverlay(false)} />
      </Show>
    </>
  );
}
