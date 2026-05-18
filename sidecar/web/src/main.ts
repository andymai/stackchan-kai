import { buildScene, setStatusTone } from "./scene";
import {
  batteryTone,
  connectStream,
  createState,
  tickPose,
  tickSaccade,
} from "./state";

const canvas = document.getElementById("scene") as HTMLCanvasElement;
const hudLink = document.getElementById("hud-link")!;
const hudEmotion = document.getElementById("hud-emotion")!;
const hudPose = document.getElementById("hud-pose")!;
const hudBattery = document.getElementById("hud-battery")!;
const hudWifi = document.getElementById("hud-wifi")!;

const scene = buildScene(canvas);
const state = createState();
connectStream("/v1/state-proxy", state);

window.addEventListener("resize", scene.resize);

let lastT = performance.now() / 1000;

function frame(): void {
  requestAnimationFrame(frame);

  const now = performance.now() / 1000;
  const dt = Math.min(now - lastT, 0.1);
  lastT = now;
  state.t = now;

  tickPose(state, dt);
  tickSaccade(state, now, dt);

  scene.head.rotation.y = state.pan;
  scene.head.rotation.x = state.tilt;

  // Breath: gentle vertical bob, ~10 mm peak-to-peak at ~6 BPM equivalent.
  const breath = Math.sin(now * 0.6) * 0.018;
  scene.scene.position.y = breath;

  // Redraw the face every frame so the microsaccade offset stays live.
  // The eye/mouth shape lookup is two object reads, cheaper than tracking
  // a "did the emotion change" guard that bypassed itself anyway.
  scene.face.draw(state.snapshot?.emotion ?? "neutral", state.eyeOffsetX, state.eyeOffsetY);
  scene.faceTexture.needsUpdate = true;

  setStatusTone(scene.status, batteryTone(state.snapshot));

  hudLink.textContent = state.conn;
  hudLink.dataset.conn = state.conn;
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

  scene.renderer.render(scene.scene, scene.camera);
}

frame();
