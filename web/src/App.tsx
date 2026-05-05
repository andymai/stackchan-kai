import { onCleanup, onMount } from "solid-js";
import { connectStream } from "./store";
import { ConnStatus } from "./components/ConnStatus";
import { State } from "./components/State";
import { Emotion } from "./components/Emotion";
import { LookAt } from "./components/LookAt";
import { Audio } from "./components/Audio";
import { Camera } from "./components/Camera";
import { Events } from "./components/Events";
import { Recovery } from "./components/Recovery";
import { Sensors } from "./components/Sensors";
import { Settings } from "./components/Settings";
import { Speak } from "./components/Speak";
import { TaskHealth } from "./components/TaskHealth";
import { Toast } from "./components/Toast";

const EMOTION_KEYS: Record<string, string> = {
  "1": "neutral",
  "2": "happy",
  "3": "sad",
  "4": "sleepy",
  "5": "surprised",
  "6": "angry",
};

export function App() {
  onMount(() => {
    connectStream();
    document.addEventListener("keydown", onKeyDown);
  });
  onCleanup(() => document.removeEventListener("keydown", onKeyDown));
  return (
    <>
      <main>
        <header>
          <h1>Stack-chan</h1>
          <ConnStatus />
        </header>
        <State />
        <Emotion />
        <LookAt />
        <Audio />
        <Speak />
        <Camera />
        <Sensors />
        <TaskHealth />
        <Events />
        <Settings />
        <Recovery />
      </main>
      <Toast />
    </>
  );
}

function onKeyDown(ev: KeyboardEvent) {
  // Don't hijack typing in form fields.
  const t = ev.target as HTMLElement | null;
  if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT")) {
    return;
  }
  if (ev.altKey || ev.ctrlKey || ev.metaKey) return;

  const k = ev.key.toLowerCase();
  const emotion = EMOTION_KEYS[ev.key];
  if (emotion) {
    const btn = document.querySelector<HTMLButtonElement>(`button[data-emotion="${emotion}"]`);
    btn?.click();
    ev.preventDefault();
    return;
  }
  if (k === "r") {
    document.querySelector<HTMLButtonElement>('button[data-shortcut="reset"]')?.click();
    ev.preventDefault();
    return;
  }
  if (k === "m") {
    document.querySelector<HTMLButtonElement>('button[data-shortcut="mute"]')?.click();
    ev.preventDefault();
  }
}
