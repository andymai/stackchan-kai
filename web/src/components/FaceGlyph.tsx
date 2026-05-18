import { Match, Switch, createMemo } from "solid-js";
import { snapshot } from "../store";

type EyeShape = "round" | "smile" | "sleepy" | "wide" | "frown" | "x" | "heart";
type MouthShape = "flat" | "smile" | "open" | "frown" | "small" | "zig";

const EYE_BY_EMOTION: Record<string, EyeShape> = {
  neutral: "round",
  happy: "smile",
  sad: "frown",
  sleepy: "sleepy",
  surprised: "wide",
  angry: "x",
  doubt: "round",
  boring: "sleepy",
  hi: "smile",
  loved: "heart",
  curious: "round",
  confused: "round",
  mad: "x",
};

const MOUTH_BY_EMOTION: Record<string, MouthShape> = {
  neutral: "flat",
  happy: "smile",
  sad: "frown",
  sleepy: "small",
  surprised: "open",
  angry: "small",
  doubt: "zig",
  boring: "flat",
  hi: "smile",
  loved: "smile",
  curious: "open",
  confused: "zig",
  mad: "frown",
};

function Eye(props: { cx: number; shape: EyeShape; fg: string; accent: string }) {
  return (
    <Switch fallback={<ellipse cx={props.cx} cy={42} rx={4.5} ry={5.5} fill={props.fg} />}>
      <Match when={props.shape === "smile"}>
        <path
          d={`M ${props.cx - 8} 42 Q ${props.cx} 32 ${props.cx + 8} 42`}
          stroke={props.fg}
          stroke-width="3"
          fill="none"
          stroke-linecap="round"
        />
      </Match>
      <Match when={props.shape === "sleepy"}>
        <line x1={props.cx - 8} y1={42} x2={props.cx + 8} y2={42} stroke={props.fg} stroke-width="3" stroke-linecap="round" />
      </Match>
      <Match when={props.shape === "wide"}>
        <circle cx={props.cx} cy={42} r={7} fill="none" stroke={props.fg} stroke-width="3" />
      </Match>
      <Match when={props.shape === "frown"}>
        <path
          d={`M ${props.cx - 8} 38 Q ${props.cx} 48 ${props.cx + 8} 38`}
          stroke={props.fg}
          stroke-width="3"
          fill="none"
          stroke-linecap="round"
        />
      </Match>
      <Match when={props.shape === "x"}>
        <g stroke={props.fg} stroke-width="3" stroke-linecap="round">
          <line x1={props.cx - 6} y1={36} x2={props.cx + 6} y2={48} />
          <line x1={props.cx - 6} y1={48} x2={props.cx + 6} y2={36} />
        </g>
      </Match>
      <Match when={props.shape === "heart"}>
        <path
          d={`M ${props.cx} 47 L ${props.cx - 7} 40 A 4 4 0 0 1 ${props.cx} 37 A 4 4 0 0 1 ${props.cx + 7} 40 Z`}
          fill={props.accent}
          stroke={props.accent}
          stroke-width="1"
          stroke-linejoin="round"
        />
      </Match>
    </Switch>
  );
}

function Mouth(props: { shape: MouthShape; fg: string; accent: string }) {
  return (
    <Switch fallback={<line x1={42} y1={70} x2={58} y2={70} stroke={props.accent} stroke-width="3" stroke-linecap="round" />}>
      <Match when={props.shape === "smile"}>
        <path d="M 38 66 Q 50 78 62 66" stroke={props.accent} stroke-width="3.5" fill="none" stroke-linecap="round" />
      </Match>
      <Match when={props.shape === "open"}>
        <ellipse cx={50} cy={70} rx={6} ry={7} fill={props.accent} />
      </Match>
      <Match when={props.shape === "frown"}>
        <path d="M 40 74 Q 50 64 60 74" stroke={props.accent} stroke-width="3.5" fill="none" stroke-linecap="round" />
      </Match>
      <Match when={props.shape === "small"}>
        <line x1={45} y1={71} x2={55} y2={71} stroke={props.accent} stroke-width="3" stroke-linecap="round" />
      </Match>
      <Match when={props.shape === "zig"}>
        <path
          d="M 40 70 L 45 67 L 50 71 L 55 67 L 60 70"
          stroke={props.accent}
          stroke-width="2.5"
          fill="none"
          stroke-linecap="round"
          stroke-linejoin="round"
        />
      </Match>
    </Switch>
  );
}

export function FaceGlyph(props: { size?: number }) {
  const size = props.size ?? 64;
  const eye = createMemo<EyeShape>(() => EYE_BY_EMOTION[snapshot()?.emotion ?? "neutral"] ?? "round");
  const mouth = createMemo<MouthShape>(() => MOUTH_BY_EMOTION[snapshot()?.emotion ?? "neutral"] ?? "flat");

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 100 100"
      role="img"
      aria-label={`face: ${snapshot()?.emotion ?? "unknown"}`}
      class="face-glyph"
    >
      <rect x={4} y={4} width={92} height={92} rx={6} fill="var(--face-bg)" stroke="var(--border)" stroke-width="1" />
      <circle cx={28} cy={62} r={4} fill="var(--face-accent)" opacity={0.7} />
      <circle cx={72} cy={62} r={4} fill="var(--face-accent)" opacity={0.7} />
      <Eye cx={35} shape={eye()} fg="var(--face-fg)" accent="var(--face-accent)" />
      <Eye cx={65} shape={eye()} fg="var(--face-fg)" accent="var(--face-accent)" />
      <Mouth shape={mouth()} fg="var(--face-fg)" accent="var(--face-accent)" />
    </svg>
  );
}
