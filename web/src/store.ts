import { createSignal } from "solid-js";
import type { AvatarSnapshot } from "./types";

export type ConnState = "connecting" | "ok" | "bad";

export const [snapshot, setSnapshot] = createSignal<AvatarSnapshot | null>(null);
export const [conn, setConn] = createSignal<ConnState>("connecting");

export type Toast = { msg: string; bad: boolean; id: number } | null;
const [toastVal, setToastVal] = createSignal<Toast>(null);
let toastTimer: ReturnType<typeof setTimeout> | null = null;
let toastSeq = 0;

export const toast = toastVal;

export function showToast(msg: string, bad = false): void {
  toastSeq += 1;
  setToastVal({ msg, bad, id: toastSeq });
  if (toastTimer != null) clearTimeout(toastTimer);
  toastTimer = setTimeout(() => setToastVal(null), 2500);
}

// Rolling window of SSE samples. ~2 min at the firmware's ~1 Hz cadence;
// the sparkline components pick whichever slice they need.
export type Sample = {
  t: number;
  battery_pct: number | null;
  audio_volume_pct: number;
  pan_deg: number;
  tilt_deg: number;
  pan_actual_deg: number | null;
  tilt_actual_deg: number | null;
};

const MAX_SAMPLES = 120;
let ring: Sample[] = [];
export const [history, setHistory] = createSignal<readonly Sample[]>([]);

function pushSample(s: AvatarSnapshot): void {
  ring.push({
    t: Date.now(),
    battery_pct: s.battery.percent,
    audio_volume_pct: s.audio.volume_pct,
    pan_deg: s.head_pose.pan_deg,
    tilt_deg: s.head_pose.tilt_deg,
    pan_actual_deg: s.head_actual?.pan_deg ?? null,
    tilt_actual_deg: s.head_actual?.tilt_deg ?? null,
  });
  if (ring.length > MAX_SAMPLES) ring = ring.slice(ring.length - MAX_SAMPLES);
  setHistory(ring.slice());
}

export function series<K extends keyof Sample>(key: K): number[] {
  const out: number[] = [];
  for (const s of history()) {
    const v = s[key];
    if (typeof v === "number") out.push(v);
  }
  return out;
}

export function connectStream(): void {
  const open = () => {
    const es = new EventSource("/state/stream");
    es.onopen = () => setConn("ok");
    es.onerror = () => {
      setConn("bad");
      es.close();
      setTimeout(open, 1500);
    };
    es.onmessage = (ev) => {
      try {
        const snap = JSON.parse(ev.data) as AvatarSnapshot;
        setSnapshot(snap);
        pushSample(snap);
      } catch {
        // SSE payloads occasionally arrive partial during reconnect; drop.
      }
    };
  };
  open();
}
