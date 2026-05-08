export type Pose = {
  pan_deg: number;
  tilt_deg: number;
};

export type BatterySnapshot = {
  percent: number | null;
  voltage_mv: number | null;
};

export type WifiSnapshot = {
  connected: boolean;
  ip: string | null;
};

export type AudioState = {
  volume_pct: number;
  muted: boolean;
};

export type AvatarSnapshot = {
  emotion: string;
  mood: string;
  face_geometry: string;
  decorator: string | null;
  head_pose: Pose;
  head_actual: Pose | null;
  battery: BatterySnapshot;
  wifi: WifiSnapshot;
  audio: AudioState;
  camera_mode: boolean;
};

export type Settings = {
  wifi: { ssid: string; psk: string; country: string };
  mdns: { hostname: string };
  time: { tz: string; sntp_servers: string[] };
  auth: { token: string };
  audio: AudioState;
};

export type Imu = {
  accel_g: [number, number, number];
  gyro_dps: [number, number, number];
};

export type BodyTouch = {
  left: number;
  centre: number;
  right: number;
};

export type SensorsSnapshot = {
  imu: Imu | null;
  ambient_lux: number | null;
  audio_rms: number;
  body_touch: BodyTouch | null;
};

export type TaskHealth = {
  name: string;
  delta: number;
  min_per_window: number;
  stale: boolean;
};

export type TasksSnapshot = {
  window_ms: number;
  channels: TaskHealth[];
};

export type EventEntry = {
  at_ms: number;
  kind: "lifecycle" | "control" | "warn";
  message: string;
};

export type EventsResponse = {
  total: number;
  events: EventEntry[];
};
