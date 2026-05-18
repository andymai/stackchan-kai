import { EMOTION_CHIPS, setEmotion, startListen } from "./actions";
import { buildScene, setStatusTone } from "./scene";
import {
  batteryTone,
  connectAgentStream,
  connectStream,
  createState,
  tickPose,
  tickSaccade,
} from "./state";

const canvas = document.getElementById("scene") as HTMLCanvasElement;
const hudLink = document.getElementById("hud-link")!;
const hudAgent = document.getElementById("hud-agent")!;
const hudEmotion = document.getElementById("hud-emotion")!;
const hudPose = document.getElementById("hud-pose")!;
const hudBattery = document.getElementById("hud-battery")!;
const hudWifi = document.getElementById("hud-wifi")!;

const thinkingEl = document.getElementById("thinking")!;
const subtitleEl = document.getElementById("subtitle")!;
const subtitleUserEl = document.getElementById("subtitle-user")!;
const subtitleReplyEl = document.getElementById("subtitle-reply")!;
const emptyEl = document.getElementById("empty-overlay")!;

const pttEl = document.getElementById("ptt") as HTMLButtonElement;
const chipsEl = document.getElementById("chips")!;

const scene = buildScene(canvas);
const state = createState();
connectStream("/v1/state-proxy", state);
connectAgentStream("/v1/session-status", state);

window.addEventListener("resize", scene.resize);

// Respect prefers-reduced-motion for the breath bob + saccade jitter.
// The pose lerp + emotion-driven face still need to track state — the
// preference affects ambient idle motion, not informative motion.
const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");

// ── Controls ──────────────────────────────────────────────────────

const chipButtons = new Map<string, HTMLButtonElement>();
for (const { id, label } of EMOTION_CHIPS) {
  const btn = document.createElement("button");
  btn.type = "button";
  btn.className = "chip";
  btn.dataset.emotion = id;
  btn.textContent = label;
  btn.setAttribute("aria-label", `Set emotion ${label}`);
  btn.setAttribute("aria-pressed", "false");
  btn.addEventListener("click", () => {
    void setEmotion(id);
  });
  chipsEl.appendChild(btn);
  chipButtons.set(id, btn);
}

let pttBusy = false;
pttEl.addEventListener("click", async () => {
  if (pttBusy) return;
  pttBusy = true;
  pttEl.classList.add("is-loading");
  pttEl.setAttribute("aria-busy", "true");
  pttEl.disabled = true;
  try {
    await startListen();
  } finally {
    pttBusy = false;
    pttEl.classList.remove("is-loading");
    pttEl.removeAttribute("aria-busy");
    pttEl.disabled = false;
  }
});

// ── Subtitle TTL ──────────────────────────────────────────────────
// Surface the latest turn for ~12 s after completion; after that hide
// so it doesn't loiter on top of the scene forever.
const SUBTITLE_TTL_MS = 12_000;
let lastTurnId: number | null = null;
let lastTurnShownAt = 0;

// ── Render loop ───────────────────────────────────────────────────

let lastChipActive = "";
let lastT = performance.now() / 1000;

function frame(): void {
  requestAnimationFrame(frame);

  const now = performance.now() / 1000;
  const dt = Math.min(now - lastT, 0.1);
  lastT = now;
  state.t = now;

  tickPose(state, dt);
  if (!reducedMotion.matches) {
    tickSaccade(state, now, dt);
  } else {
    state.eyeOffsetX = 0;
    state.eyeOffsetY = 0;
  }

  scene.head.rotation.y = state.pan;
  scene.head.rotation.x = state.tilt;

  // Breath: gentle vertical bob, ~10 mm peak-to-peak. Skipped when the
  // operator opted out of idle motion.
  scene.scene.position.y = reducedMotion.matches ? 0 : Math.sin(now * 0.6) * 0.018;

  // Redraw the face every frame so the microsaccade offset stays live.
  // The eye/mouth shape lookup is two object reads, cheaper than tracking
  // a "did the emotion change" guard that bypassed itself anyway.
  scene.face.draw(state.snapshot?.emotion ?? "neutral", state.eyeOffsetX, state.eyeOffsetY);
  scene.faceTexture.needsUpdate = true;

  setStatusTone(scene.status, batteryTone(state.snapshot));

  // ── HUD ────────────────────────────────────────────────────────
  hudLink.textContent = state.conn;
  hudLink.dataset.conn = state.conn;
  const agentLabel =
    state.agentConn === "live"
      ? state.agent?.state === "thinking"
        ? "thinking"
        : "idle"
      : state.agentConn;
  hudAgent.textContent = agentLabel;
  hudAgent.dataset.conn = state.agentConn;
  hudAgent.dataset.state = state.agent?.state ?? "";

  hudEmotion.textContent = (state.snapshot?.emotion ?? "—").toUpperCase();
  if (state.snapshot) {
    const p = state.snapshot.head_actual ?? state.snapshot.head_pose;
    hudPose.textContent = `${p.pan_deg.toFixed(0)}° / ${p.tilt_deg.toFixed(0)}°`;
    const b = state.snapshot.battery;
    hudBattery.textContent = b.percent != null ? `${b.percent}%` : "—";
    hudWifi.textContent = state.snapshot.wifi.connected
      ? (state.snapshot.wifi.ip ?? "up")
      : "down";
  } else {
    hudPose.textContent = "—";
    hudBattery.textContent = "—";
    hudWifi.textContent = "—";
  }

  // ── Empty overlay ─────────────────────────────────────────────
  // Surfaces a hint when neither stream has produced a sample yet so
  // a fresh visitor doesn't stare at a frozen face wondering whether
  // the firmware is reachable.
  const noStreams = state.snapshot == null && state.agent == null;
  emptyEl.classList.toggle("hidden", !noStreams);

  // ── Thinking indicator ─────────────────────────────────────────
  const thinking = state.agent?.state === "thinking";
  thinkingEl.classList.toggle("hidden", !thinking);

  // ── Active emotion chip ────────────────────────────────────────
  const activeEmotion = state.snapshot?.emotion ?? "";
  if (activeEmotion !== lastChipActive) {
    const prev = chipButtons.get(lastChipActive);
    const curr = chipButtons.get(activeEmotion);
    if (prev) {
      prev.classList.remove("active");
      prev.setAttribute("aria-pressed", "false");
    }
    if (curr) {
      curr.classList.add("active");
      curr.setAttribute("aria-pressed", "true");
    }
    lastChipActive = activeEmotion;
  }

  // ── Subtitle TTL handling ──────────────────────────────────────
  const turn = state.agent?.last_turn;
  if (turn) {
    if (turn.completed_at !== lastTurnId) {
      lastTurnId = turn.completed_at;
      lastTurnShownAt = performance.now();
      subtitleUserEl.textContent = turn.transcript || "(silence)";
      subtitleReplyEl.textContent = turn.reply_short;
    }
    const age = performance.now() - lastTurnShownAt;
    subtitleEl.classList.toggle("hidden", age > SUBTITLE_TTL_MS);
  } else {
    subtitleEl.classList.add("hidden");
  }

  scene.renderer.render(scene.scene, scene.camera);
}

frame();
