import { createMemo } from "solid-js";
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
  const { cx, shape, fg, accent } = props;
  if (shape === "smile") {
    return <path d={`M ${cx - 8} 42 Q ${cx} 32 ${cx + 8} 42`} stroke={fg} stroke-width="3" fill="none" stroke-linecap="round" />;
  }
  if (shape === "sleepy") {
    return <line x1={cx - 8} y1={42} x2={cx + 8} y2={42} stroke={fg} stroke-width="3" stroke-linecap="round" />;
  }
  if (shape === "wide") {
    return <circle cx={cx} cy={42} r={7} fill="none" stroke={fg} stroke-width="3" />;
  }
  if (shape === "frown") {
    return <path d={`M ${cx - 8} 38 Q ${cx} 48 ${cx + 8} 38`} stroke={fg} stroke-width="3" fill="none" stroke-linecap="round" />;
  }
  if (shape === "x") {
    return (
      <g stroke={fg} stroke-width="3" stroke-linecap="round">
        <line x1={cx - 6} y1={36} x2={cx + 6} y2={48} />
        <line x1={cx - 6} y1={48} x2={cx + 6} y2={36} />
      </g>
    );
  }
  if (shape === "heart") {
    return (
      <path
        d={`M ${cx} 47 L ${cx - 7} 40 A 4 4 0 0 1 ${cx} 37 A 4 4 0 0 1 ${cx + 7} 40 Z`}
        fill={accent}
        stroke={accent}
        stroke-width="1"
        stroke-linejoin="round"
      />
    );
  }
  return <ellipse cx={cx} cy={42} rx={4.5} ry={5.5} fill={fg} />;
}

function Mouth(props: { shape: MouthShape; fg: string; accent: string }) {
  const { shape, fg, accent } = props;
  if (shape === "smile") {
    return <path d="M 38 66 Q 50 78 62 66" stroke={accent} stroke-width="3.5" fill="none" stroke-linecap="round" />;
  }
  if (shape === "open") {
    return <ellipse cx={50} cy={70} rx={6} ry={7} fill={accent} />;
  }
  if (shape === "frown") {
    return <path d="M 40 74 Q 50 64 60 74" stroke={accent} stroke-width="3.5" fill="none" stroke-linecap="round" />;
  }
  if (shape === "small") {
    return <line x1={45} y1={71} x2={55} y2={71} stroke={accent} stroke-width="3" stroke-linecap="round" />;
  }
  if (shape === "zig") {
    return (
      <path
        d="M 40 70 L 45 67 L 50 71 L 55 67 L 60 70"
        stroke={accent}
        stroke-width="2.5"
        fill="none"
        stroke-linecap="round"
        stroke-linejoin="round"
      />
    );
  }
  return <line x1={42} y1={70} x2={58} y2={70} stroke={accent} stroke-width="3" stroke-linecap="round" />;
}

export function FaceGlyph(props: { size?: number }) {
  const size = props.size ?? 64;
  const state = createMemo(() => {
    const s = snapshot();
    const emo = s?.emotion ?? "neutral";
    return {
      eye: EYE_BY_EMOTION[emo] ?? "round",
      mouth: MOUTH_BY_EMOTION[emo] ?? "flat",
      bg: "var(--face-bg)",
      fg: "var(--face-fg)",
      accent: "var(--face-accent)",
    };
  });

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 100 100"
      role="img"
      aria-label={`face: ${snapshot()?.emotion ?? "unknown"}`}
      class="face-glyph"
    >
      <rect x={4} y={4} width={92} height={92} rx={6} fill={state().bg} stroke="var(--border)" stroke-width="1" />
      <circle cx={28} cy={62} r={4} fill={state().accent} opacity={0.7} />
      <circle cx={72} cy={62} r={4} fill={state().accent} opacity={0.7} />
      <Eye cx={35} shape={state().eye} fg={state().fg} accent={state().accent} />
      <Eye cx={65} shape={state().eye} fg={state().fg} accent={state().accent} />
      <Mouth shape={state().mouth} fg={state().fg} accent={state().accent} />
    </svg>
  );
}
