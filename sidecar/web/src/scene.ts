import {
  AmbientLight,
  BoxGeometry,
  CanvasTexture,
  Color,
  CylinderGeometry,
  DirectionalLight,
  Group,
  Mesh,
  MeshBasicMaterial,
  MeshStandardMaterial,
  PerspectiveCamera,
  PointLight,
  Scene,
  SphereGeometry,
  SRGBColorSpace,
  WebGLRenderer,
} from "three";
import { createFacePainter, type FacePainter } from "./face-texture";

const BODY_COLOR = 0x1a1d22;
const FACE_BG = 0xf6f3e8;
const ACCENT = 0xf08080;

export type SceneHandles = {
  scene: Scene;
  camera: PerspectiveCamera;
  renderer: WebGLRenderer;
  head: Group;
  face: FacePainter;
  faceTexture: CanvasTexture;
  status: Mesh;
  resize: () => void;
  dispose: () => void;
};

export function buildScene(canvas: HTMLCanvasElement): SceneHandles {
  const scene = new Scene();
  scene.background = new Color(0x0b0e13);

  const camera = new PerspectiveCamera(32, 1, 0.1, 50);
  camera.position.set(0, 0.6, 6.2);
  camera.lookAt(0, 0.3, 0);

  const renderer = new WebGLRenderer({ canvas, antialias: true, alpha: false });
  renderer.outputColorSpace = SRGBColorSpace;
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));

  // Lighting — ambient + a key with a coral rim that nods to Stack-chan's cheek.
  scene.add(new AmbientLight(0xffffff, 0.55));
  const key = new DirectionalLight(0xffffff, 0.9);
  key.position.set(4, 6, 5);
  scene.add(key);
  const rim = new PointLight(ACCENT, 0.35, 12);
  rim.position.set(-3, 2, -1.5);
  scene.add(rim);

  // Body (CoreS3 chassis). Slightly taller than wide.
  const bodyMat = new MeshStandardMaterial({ color: BODY_COLOR, roughness: 0.62, metalness: 0.08 });
  const body = new Mesh(new BoxGeometry(1.5, 1.45, 1.0), bodyMat);
  body.position.y = -0.55;
  scene.add(body);

  // Status LED on the chassis front — tinted by /state battery + wifi.
  const statusMat = new MeshBasicMaterial({ color: 0x5fc88f });
  const status = new Mesh(new SphereGeometry(0.07, 16, 16), statusMat);
  status.position.set(0.55, -0.95, 0.51);
  scene.add(status);

  // Neck post — small cylinder bridging body and head pivot.
  const neckMat = new MeshStandardMaterial({ color: 0x2a2e36, roughness: 0.7 });
  const neck = new Mesh(new CylinderGeometry(0.12, 0.16, 0.32, 16), neckMat);
  neck.position.y = 0.1;
  scene.add(neck);

  // Head group — pan = rotation.y, tilt = rotation.x. Pivot at neck top.
  const head = new Group();
  head.position.y = 0.55;
  scene.add(head);

  const headBody = new Mesh(new BoxGeometry(1.45, 1.05, 0.95), bodyMat);
  head.add(headBody);

  // Face decal: a slightly inset plane in front of the head box.
  const face = createFacePainter();
  const faceTexture = new CanvasTexture(face.canvas);
  faceTexture.colorSpace = SRGBColorSpace;
  faceTexture.needsUpdate = true;

  const faceMat = new MeshStandardMaterial({
    color: FACE_BG,
    map: faceTexture,
    roughness: 0.35,
    metalness: 0.0,
    emissive: 0xffffff,
    emissiveMap: faceTexture,
    emissiveIntensity: 0.18,
  });
  const facePlane = new Mesh(new BoxGeometry(1.18, 0.85, 0.02), faceMat);
  facePlane.position.set(0, 0, 0.485);
  head.add(facePlane);

  // Two tiny coral accents at the chassis bottom edge — a nod to the
  // brand-mark on the dashboard's sidebar.
  const accentMat = new MeshBasicMaterial({ color: ACCENT });
  const leftDot = new Mesh(new SphereGeometry(0.04, 10, 10), accentMat);
  leftDot.position.set(-0.55, -1.18, 0.51);
  scene.add(leftDot);
  const rightDot = leftDot.clone();
  rightDot.position.x = 0.55;
  scene.add(rightDot);

  const resize = (): void => {
    const parent = canvas.parentElement;
    const w = parent?.clientWidth ?? window.innerWidth;
    const h = parent?.clientHeight ?? window.innerHeight;
    renderer.setSize(w, h, false);
    camera.aspect = w / h;
    camera.updateProjectionMatrix();
  };
  resize();

  const dispose = (): void => {
    renderer.dispose();
    faceTexture.dispose();
  };

  return { scene, camera, renderer, head, face, faceTexture, status, resize, dispose };
}

export function setStatusTone(mesh: Mesh, tone: "ok" | "warn" | "bad"): void {
  const mat = mesh.material as MeshBasicMaterial;
  const hex = tone === "ok" ? 0x5fc88f : tone === "warn" ? 0xe6a155 : 0xe2625a;
  mat.color.setHex(hex);
}
