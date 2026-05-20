import { createSignal } from "solid-js";
import type { AvatarSnapshot } from "./types";

export type ConnState = "connecting" | "ok" | "bad";

export const [snapshot, setSnapshot] = createSignal<AvatarSnapshot | null>(null);
export const [conn, setConn] = createSignal<ConnState>("connecting");

export type Toast = { msg: string; bad: boolean; id: number };
const [toastList, setToastList] = createSignal<readonly Toast[]>([]);
let toastSeq = 0;

const MAX_TOASTS = 3;
const TOAST_TTL_MS = 2800;

export const toasts = toastList;

export function showToast(msg: string, bad = false): void {
  toastSeq += 1;
  const id = toastSeq;
  setToastList((curr) => {
    const next = [...curr, { msg, bad, id }];
    // Drop oldest if we've stacked beyond the cap so the screen doesn't
    // fill up during a burst of failures.
    return next.length > MAX_TOASTS ? next.slice(next.length - MAX_TOASTS) : next;
  });
  setTimeout(() => {
    setToastList((curr) => curr.filter((t) => t.id !== id));
  }, TOAST_TTL_MS);
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
  // Exponential backoff with jitter for reconnects. Fixed 1.5 s
  // before this change meant every open dashboard tab pounded the
  // firmware at 0.67 Hz whenever it was offline; bursts from N
  // tabs would amplify under restart. Resets to the floor on
  // every successful open so a healthy stream re-arms the curve.
  const BACKOFF_MIN_MS = 1500;
  const BACKOFF_MAX_MS = 30000;
  let backoff = BACKOFF_MIN_MS;
  const open = () => {
    const es = new EventSource("/state/stream");
    es.onopen = () => {
      setConn("ok");
      backoff = BACKOFF_MIN_MS;
    };
    es.onerror = () => {
      setConn("bad");
      es.close();
      // ±25% jitter so N tabs reconnecting after a firmware
      // restart fan out across the back-off window instead of
      // synchronising on the boundary.
      const jitter = backoff * (0.75 + Math.random() * 0.5);
      setTimeout(open, jitter);
      backoff = Math.min(backoff * 2, BACKOFF_MAX_MS);
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
