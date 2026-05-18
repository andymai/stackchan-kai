import { Show, createMemo } from "solid-js";

type Props = {
  values: readonly number[];
  width?: number;
  height?: number;
  min?: number;
  max?: number;
  stroke?: string;
  fill?: string;
  zeroLine?: boolean;
};

export function Sparkline(props: Props) {
  const width = () => props.width ?? 120;
  const height = () => props.height ?? 28;

  const geom = createMemo(() => {
    const vs = props.values;
    if (vs.length === 0) return null;

    const lo = props.min ?? Math.min(...vs);
    const hi = props.max ?? Math.max(...vs);
    const span = hi - lo || 1;
    const w = width();
    const h = height();
    const stepX = vs.length > 1 ? w / (vs.length - 1) : 0;

    let path = "";
    let area = `M 0 ${h} `;
    vs.forEach((v, i) => {
      const x = i * stepX;
      const y = h - ((v - lo) / span) * (h - 2) - 1;
      const cmd = i === 0 ? "M" : "L";
      path += `${cmd} ${x.toFixed(1)} ${y.toFixed(1)} `;
      area += `L ${x.toFixed(1)} ${y.toFixed(1)} `;
    });
    area += `L ${w} ${h} Z`;
    return { path, area, w, h, lo, hi };
  });

  return (
    <Show
      when={geom()}
      fallback={
        <svg width={width()} height={height()} class="sparkline sparkline-empty" aria-hidden="true">
          <line x1={0} y1={height() / 2} x2={width()} y2={height() / 2} class="sparkline-axis" />
        </svg>
      }
    >
      {(g) => (
        <svg
          width={g().w}
          height={g().h}
          viewBox={`0 0 ${g().w} ${g().h}`}
          class="sparkline"
          aria-hidden="true"
        >
          <Show when={props.fill !== undefined}>
            <path d={g().area} fill={props.fill} />
          </Show>
          <path d={g().path} fill="none" stroke={props.stroke ?? "var(--accent)"} stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      )}
    </Show>
  );
}
