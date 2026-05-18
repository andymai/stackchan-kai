import { Show, createMemo, createSignal } from "solid-js";
import { postJson } from "../auth";
import { showToast, snapshot } from "../store";

const PAN_RANGE = 60;
const TILT_RANGE = 30;
const HOLD_MS = 30_000;

const PAD_W = 320;
const PAD_H = 200;

type Pos = { pan: number; tilt: number };

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));

function padToPose(x: number, y: number, rect: DOMRect): Pos {
  const fx = clamp((x - rect.left) / rect.width, 0, 1);
  const fy = clamp((y - rect.top) / rect.height, 0, 1);
  return {
    pan: Math.round((fx * 2 - 1) * PAN_RANGE),
    tilt: Math.round((1 - fy * 2) * TILT_RANGE),
  };
}

function poseToPad(p: Pos): { x: number; y: number } {
  return {
    x: ((p.pan / PAN_RANGE) * 0.5 + 0.5) * PAD_W,
    y: (1 - ((p.tilt / TILT_RANGE) * 0.5 + 0.5)) * PAD_H,
  };
}

export function PosePad() {
  const [target, setTarget] = createSignal<Pos>({ pan: 0, tilt: 0 });
  const [dragging, setDragging] = createSignal(false);
  let svgEl: SVGSVGElement | undefined;

  const commanded = createMemo<Pos>(() => {
    if (dragging()) return target();
    const s = snapshot();
    if (!s) return target();
    return { pan: Math.round(s.head_pose.pan_deg), tilt: Math.round(s.head_pose.tilt_deg) };
  });

  const actual = createMemo<Pos | null>(() => {
    const a = snapshot()?.head_actual;
    return a ? { pan: a.pan_deg, tilt: a.tilt_deg } : null;
  });

  const send = async (p: Pos) => {
    try {
      await postJson("/look-at", { pan_deg: p.pan, tilt_deg: p.tilt, hold_ms: HOLD_MS });
      showToast(`look-at ${p.pan}°, ${p.tilt}°`);
    } catch (e) {
      showToast((e as Error).message, true);
    }
  };

  const onPointerDown = (e: PointerEvent) => {
    if (!svgEl) return;
    svgEl.setPointerCapture(e.pointerId);
    setDragging(true);
    setTarget(padToPose(e.clientX, e.clientY, svgEl.getBoundingClientRect()));
  };

  const onPointerMove = (e: PointerEvent) => {
    if (!dragging() || !svgEl) return;
    setTarget(padToPose(e.clientX, e.clientY, svgEl.getBoundingClientRect()));
  };

  const onPointerUp = () => {
    if (!dragging()) return;
    setDragging(false);
    void send(target());
  };

  const center = () => {
    setTarget({ pan: 0, tilt: 0 });
    void send({ pan: 0, tilt: 0 });
  };

  const cmdXY = createMemo(() => poseToPad(commanded()));
  const actXY = createMemo(() => {
    const a = actual();
    return a ? poseToPad(a) : null;
  });

  return (
    <div class="pose-pad-wrap">
      <svg
        ref={svgEl}
        viewBox={`0 0 ${PAD_W} ${PAD_H}`}
        class="pose-pad"
        preserveAspectRatio="xMidYMid meet"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        role="application"
        aria-label="Head look-at pad"
      >
        <rect x={0} y={0} width={PAD_W} height={PAD_H} class="pad-bg" />

        {Array.from({ length: 7 }, (_, i) => i - 3).map((i) => (
          <line
            x1={((i + 3) / 6) * PAD_W}
            y1={0}
            x2={((i + 3) / 6) * PAD_W}
            y2={PAD_H}
            class="pad-grid"
          />
        ))}
        {Array.from({ length: 7 }, (_, i) => i - 3).map((i) => (
          <line
            x1={0}
            y1={((i + 3) / 6) * PAD_H}
            x2={PAD_W}
            y2={((i + 3) / 6) * PAD_H}
            class="pad-grid"
          />
        ))}

        <line x1={PAD_W / 2} y1={0} x2={PAD_W / 2} y2={PAD_H} class="pad-axis" />
        <line x1={0} y1={PAD_H / 2} x2={PAD_W} y2={PAD_H / 2} class="pad-axis" />

        <Show when={actXY()}>
          {(p) => (
            <g>
              <circle cx={p().x} cy={p().y} r={8} class="pad-actual" />
              <line x1={cmdXY().x} y1={cmdXY().y} x2={p().x} y2={p().y} class="pad-link" />
            </g>
          )}
        </Show>

        <g class="pad-cmd">
          <circle cx={cmdXY().x} cy={cmdXY().y} r={5} />
          <circle cx={cmdXY().x} cy={cmdXY().y} r={11} class="pad-cmd-halo" />
        </g>

        <text x={8} y={16} class="pad-label">PAN</text>
        <text x={PAD_W - 8} y={16} class="pad-label pad-label-r">
          {commanded().pan}°
        </text>
        <text x={8} y={PAD_H - 8} class="pad-label">TILT {commanded().tilt}°</text>
      </svg>
      <button type="button" class="pad-center" onClick={center}>Center</button>
    </div>
  );
}
