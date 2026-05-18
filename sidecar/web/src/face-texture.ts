// Port of the dashboard's schematic FaceGlyph from SVG to canvas 2D so we
// can bind it as a THREE.CanvasTexture and project it onto the head's
// front face. Shapes match the SVG one-to-one — see `web/src/components/FaceGlyph.tsx`.

export type EyeShape = "round" | "smile" | "sleepy" | "wide" | "frown" | "x" | "heart";
export type MouthShape = "flat" | "smile" | "open" | "frown" | "small" | "zig";

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

const FACE_BG = "#f6f3e8";
const FACE_FG = "#0d1014";
const FACE_ACCENT = "#f08080";

const W = 256;
const H = 256;
const CX_L = W * 0.35;
const CX_R = W * 0.65;
const CY_EYE = H * 0.44;
const CY_MOUTH = H * 0.72;

function drawEye(ctx: CanvasRenderingContext2D, cx: number, eyeOffsetX: number, eyeOffsetY: number, shape: EyeShape): void {
  const x = cx + eyeOffsetX;
  const y = CY_EYE + eyeOffsetY;
  ctx.save();
  ctx.lineWidth = 8;
  ctx.lineCap = "round";
  ctx.strokeStyle = FACE_FG;
  ctx.fillStyle = FACE_FG;
  switch (shape) {
    case "round":
      ctx.beginPath();
      ctx.ellipse(x, y, 11, 14, 0, 0, Math.PI * 2);
      ctx.fill();
      break;
    case "smile":
      ctx.beginPath();
      ctx.moveTo(x - 20, y);
      ctx.quadraticCurveTo(x, y - 25, x + 20, y);
      ctx.stroke();
      break;
    case "sleepy":
      ctx.beginPath();
      ctx.moveTo(x - 20, y);
      ctx.lineTo(x + 20, y);
      ctx.stroke();
      break;
    case "wide":
      ctx.beginPath();
      ctx.arc(x, y, 17, 0, Math.PI * 2);
      ctx.stroke();
      break;
    case "frown":
      ctx.beginPath();
      ctx.moveTo(x - 20, y - 12);
      ctx.quadraticCurveTo(x, y + 13, x + 20, y - 12);
      ctx.stroke();
      break;
    case "x":
      ctx.beginPath();
      ctx.moveTo(x - 14, y - 14);
      ctx.lineTo(x + 14, y + 14);
      ctx.moveTo(x - 14, y + 14);
      ctx.lineTo(x + 14, y - 14);
      ctx.stroke();
      break;
    case "heart": {
      ctx.fillStyle = FACE_ACCENT;
      ctx.strokeStyle = FACE_ACCENT;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.moveTo(x, y + 12);
      ctx.lineTo(x - 17, y - 5);
      ctx.arc(x - 8.5, y - 8.5, 9, Math.PI, Math.PI * 1.5);
      ctx.arc(x + 8.5, y - 8.5, 9, Math.PI * 1.5, Math.PI * 2);
      ctx.closePath();
      ctx.fill();
      break;
    }
  }
  ctx.restore();
}

function drawMouth(ctx: CanvasRenderingContext2D, shape: MouthShape): void {
  const x = W / 2;
  const y = CY_MOUTH;
  ctx.save();
  ctx.lineWidth = 9;
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.strokeStyle = FACE_ACCENT;
  ctx.fillStyle = FACE_ACCENT;
  switch (shape) {
    case "smile":
      ctx.beginPath();
      ctx.moveTo(x - 32, y - 8);
      ctx.quadraticCurveTo(x, y + 24, x + 32, y - 8);
      ctx.stroke();
      break;
    case "open":
      ctx.beginPath();
      ctx.ellipse(x, y + 6, 16, 18, 0, 0, Math.PI * 2);
      ctx.fill();
      break;
    case "frown":
      ctx.beginPath();
      ctx.moveTo(x - 30, y + 16);
      ctx.quadraticCurveTo(x, y - 16, x + 30, y + 16);
      ctx.stroke();
      break;
    case "small":
      ctx.lineWidth = 7;
      ctx.beginPath();
      ctx.moveTo(x - 14, y);
      ctx.lineTo(x + 14, y);
      ctx.stroke();
      break;
    case "zig":
      ctx.lineWidth = 6;
      ctx.beginPath();
      ctx.moveTo(x - 28, y);
      ctx.lineTo(x - 14, y - 10);
      ctx.lineTo(x, y + 4);
      ctx.lineTo(x + 14, y - 10);
      ctx.lineTo(x + 28, y);
      ctx.stroke();
      break;
    case "flat":
    default:
      ctx.beginPath();
      ctx.moveTo(x - 24, y);
      ctx.lineTo(x + 24, y);
      ctx.stroke();
      break;
  }
  ctx.restore();
}

function drawCheeks(ctx: CanvasRenderingContext2D): void {
  ctx.save();
  ctx.fillStyle = FACE_ACCENT;
  ctx.globalAlpha = 0.65;
  ctx.beginPath();
  ctx.arc(W * 0.22, H * 0.62, 12, 0, Math.PI * 2);
  ctx.arc(W * 0.78, H * 0.62, 12, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
}

export type FacePainter = {
  canvas: HTMLCanvasElement;
  draw: (emotion: string, eyeOffsetX?: number, eyeOffsetY?: number) => void;
};

export function createFacePainter(): FacePainter {
  const canvas = document.createElement("canvas");
  canvas.width = W;
  canvas.height = H;
  const ctx = canvas.getContext("2d")!;

  const draw = (emotion: string, eyeOffsetX = 0, eyeOffsetY = 0): void => {
    ctx.save();
    ctx.fillStyle = FACE_BG;
    ctx.fillRect(0, 0, W, H);
    drawCheeks(ctx);
    const eye = EYE_BY_EMOTION[emotion] ?? "round";
    const mouth = MOUTH_BY_EMOTION[emotion] ?? "flat";
    drawEye(ctx, CX_L, eyeOffsetX, eyeOffsetY, eye);
    drawEye(ctx, CX_R, eyeOffsetX, eyeOffsetY, eye);
    drawMouth(ctx, mouth);
    ctx.restore();
  };

  draw("neutral");
  return { canvas, draw };
}
