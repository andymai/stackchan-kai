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
