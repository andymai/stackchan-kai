import { onCleanup, onMount } from "solid-js";
import { connectStream } from "./store";
import { ConnStatus } from "./components/ConnStatus";
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
import { Toast } from "./components/Toast";

// Keys 1-6 stay pinned to the original palette (muscle memory). The
// expanded set fills the next contiguous keys: 7-9 + 0 cover four,
// q/w/e cover the last three.
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
        <CrashBanner />
        <State />
        <Emotion />
        <Mood />
        <FaceGeometry />
        <LookAt />
        <Audio />
        <Speak />
        <Listen />
        <Pair />
        <Camera />
        <Calibration />
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
  // Lowercase lookup so `q/w/e` work regardless of shift state, mirroring
  // how R/M work below. Digits are unaffected by `.toLowerCase()`.
  const emotion = EMOTION_KEYS[k];
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
