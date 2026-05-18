// Types mirror crates/stackchan-core's AvatarSnapshot wire format.
export type Pose = { pan_deg: number; tilt_deg: number };

// Voice agent state — wire format from /v1/session-status on the sidecar.
export type Turn = {
  transcript: string;
  reply_short: string;
  emotion: string;
  completed_at: number;
};

export type SessionStatus = {
  state: "idle" | "thinking";
  request_id?: string | null;
  session_id?: string | null;
  last_turn?: Turn;
  error?: string;
  updated_at: number;
};

export type AvatarSnapshot = {
  emotion: string;
  mood: string;
  face_geometry: string;
  decorator: string | null;
  head_pose: Pose;
  head_actual: Pose | null;
  battery: { percent: number | null; voltage_mv: number | null };
  wifi: { connected: boolean; ip: string | null };
  audio: { volume_pct: number; muted: boolean };
  camera_mode: boolean;
};

export type ConnState = "linking" | "live" | "down";

export type StateModel = {
  snapshot: AvatarSnapshot | null;
  conn: ConnState;
  agentConn: ConnState;
  agent: SessionStatus | null;
  // Smoothed view-state — radians, lerped toward target each frame.
  pan: number;
  tilt: number;
  // Microsaccade eye offset on the face decal (pixels).
  eyeOffsetX: number;
  eyeOffsetY: number;
  eyeTargetX: number;
  eyeTargetY: number;
  nextSaccadeAt: number;
  // Idle breath phase, drives a tiny body bob.
  t: number;
};

const DEG = Math.PI / 180;

const SACCADE_MIN_S = 0.5;
const SACCADE_MAX_S = 1.5;
const SACCADE_AMPL_PX = 1.6;
const LERP_RATE = 6.0;

export function createState(): StateModel {
  return {
    snapshot: null,
    conn: "linking",
    agentConn: "linking",
    agent: null,
    pan: 0,
    tilt: 0,
    eyeOffsetX: 0,
    eyeOffsetY: 0,
    eyeTargetX: 0,
    eyeTargetY: 0,
    nextSaccadeAt: 0,
    t: 0,
  };
}

function connectSse<T>(
  url: string,
  onMessage: (data: T) => void,
  setConn: (s: ConnState) => void,
): () => void {
  let es: EventSource | null = null;
  let retry: ReturnType<typeof setTimeout> | null = null;
  let closed = false;

  const open = (): void => {
    if (closed) return;
    es = new EventSource(url);
    es.onopen = () => setConn("live");
    es.onmessage = (ev) => {
      try {
        onMessage(JSON.parse(ev.data) as T);
      } catch {
        // partial payloads can land during reconnect — drop quietly
      }
    };
    es.onerror = () => {
      setConn("down");
      es?.close();
      es = null;
      if (!closed) retry = setTimeout(open, 1500);
    };
  };

  open();
  return () => {
    closed = true;
    if (retry != null) clearTimeout(retry);
    es?.close();
  };
}

export function connectStream(url: string, model: StateModel): () => void {
  return connectSse<AvatarSnapshot>(
    url,
    (snap) => {
      model.snapshot = snap;
    },
    (c) => {
      model.conn = c;
    },
  );
}

export function connectAgentStream(url: string, model: StateModel): () => void {
  return connectSse<SessionStatus>(
    url,
    (s) => {
      model.agent = s;
    },
    (c) => {
      model.agentConn = c;
    },
  );
}

export function targetPose(model: StateModel): Pose {
  const s = model.snapshot;
  if (!s) return { pan_deg: 0, tilt_deg: 0 };
  // head_actual reflects the SCServos' reported position; head_pose is the
  // commanded target. Mirror actual so the model lags reality the way the
  // physical robot does — feels more alive than tracking the command.
  return s.head_actual ?? s.head_pose;
}

export function batteryTone(s: AvatarSnapshot | null): "ok" | "warn" | "bad" {
  if (!s) return "bad";
  if (!s.wifi.connected) return "bad";
  const p = s.battery.percent;
  if (p == null) return "warn";
  if (p < 15) return "bad";
  if (p < 40) return "warn";
  return "ok";
}

// Lerp current pan/tilt toward the snapshot's pose; spring-like.
export function tickPose(model: StateModel, dt: number): void {
  const t = targetPose(model);
  const k = 1 - Math.exp(-LERP_RATE * dt);
  // Stack-chan: +pan = head right (operator POV), +tilt = head up. Three.js
  // camera looks from +Z toward origin, so +pan maps to -rotation.y and
  // +tilt maps to +rotation.x.
  model.pan += ((-t.pan_deg * DEG) - model.pan) * k;
  model.tilt += ((t.tilt_deg * DEG) - model.tilt) * k;
}

export function tickSaccade(model: StateModel, now: number, dt: number): void {
  if (now >= model.nextSaccadeAt) {
    model.eyeTargetX = (Math.random() - 0.5) * 2 * SACCADE_AMPL_PX;
    model.eyeTargetY = (Math.random() - 0.5) * 2 * SACCADE_AMPL_PX;
    model.nextSaccadeAt = now + SACCADE_MIN_S + Math.random() * (SACCADE_MAX_S - SACCADE_MIN_S);
  }
  const k = 1 - Math.exp(-10 * dt);
  model.eyeOffsetX += (model.eyeTargetX - model.eyeOffsetX) * k;
  model.eyeOffsetY += (model.eyeTargetY - model.eyeOffsetY) * k;
}
