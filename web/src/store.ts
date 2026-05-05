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
        setSnapshot(JSON.parse(ev.data) as AvatarSnapshot);
      } catch {
        // SSE payloads occasionally arrive partial during reconnect; drop.
      }
    };
  };
  open();
}
