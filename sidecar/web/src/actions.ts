// Companion → sidecar → firmware command relays. The companion never
// holds the firmware bearer token; the sidecar injects it before the
// proxied POST.

async function firmwareCmd(name: string, body: unknown): Promise<Response> {
  const r = await fetch(`/v1/firmware-cmd/${name}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body ?? {}),
  });
  return r;
}

const DEFAULT_LISTEN_MS = 5000;
const DEFAULT_HOLD_MS = 30_000;

export async function startListen(durationMs: number = DEFAULT_LISTEN_MS): Promise<void> {
  await firmwareCmd("listen", { duration_ms: durationMs });
}

export async function setEmotion(
  emotion: string,
  holdMs: number = DEFAULT_HOLD_MS,
): Promise<void> {
  await firmwareCmd("emotion", { emotion, hold_ms: holdMs });
}

export const EMOTION_CHIPS: readonly { id: string; label: string }[] = [
  { id: "neutral", label: "Neutral" },
  { id: "happy", label: "Happy" },
  { id: "sad", label: "Sad" },
  { id: "sleepy", label: "Sleepy" },
  { id: "surprised", label: "Surprised" },
  { id: "angry", label: "Angry" },
  { id: "loved", label: "Loved" },
  { id: "curious", label: "Curious" },
] as const;
