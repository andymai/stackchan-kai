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

export type Tracker = {
  fov_h_deg: number;
  fov_v_deg: number;
  target_smoothing_alpha: number;
  flip_x: boolean;
  flip_y: boolean;
};

// Mirrors `stackchan_net::config::TrackerSettings::DEFAULT`. The
// dashboard only consults this as a fallback when /settings is
// unreachable (e.g. SD card absent); on a healthy device the values
// always come from the firmware's GET /settings response.
export const TRACKER_DEFAULT: Tracker = {
  fov_h_deg: 62.0,
  fov_v_deg: 49.0,
  target_smoothing_alpha: 1.0,
  flip_x: false,
  flip_y: false,
};

export type HeadOffsets = {
  yaw_offset_deg: number;
  tilt_offset_deg: number;
};

export type Settings = {
  wifi: { ssid: string; psk: string; country: string };
  mdns: { hostname: string };
  time: { tz: string; sntp_servers: string[] };
  auth: { token: string };
  audio: AudioState;
  tracker: Tracker;
  esp_now: EspNow;
  behavior: Behavior;
  head: HeadTrim;
  appearance: Appearance;
};

// Per-unit head zero-point trim, applied at boot. Mirrors the
// `head` block emitted by `render_settings_json` in
// `crates/stackchan-net/src/bare_json.rs`.
export type HeadTrim = {
  pan_trim_deg: number;
  tilt_trim_deg: number;
};

// Boot-pinned default appearance. Empty strings mean "not pinned" —
// the firmware falls back to the variant default. Wire vocabularies
// mirror `Palette::wire_str` / `FaceGeometry::wire_str` in
// `crates/stackchan-core/src/{palette,face}.rs`.
export type Appearance = {
  palette: string;
  face_geometry: string;
};

export type EspNow = {
  enabled: boolean;
  pmk_hex: string;
  peer_mac: string;
  lmk_hex: string;
  channel: number | null;
  tx_rate_hz: number;
};

export type Behavior = {
  soliloquy_enabled: boolean;
  hourly_chime_enabled: boolean;
  battery_icon_enabled: boolean;
  toast_overlay_enabled: boolean;
  auto_torque_release_ms: number;
  audio_debug_udp_target: string;
  agent_sidecar_url: string;
  agent_sidecar_token: string;
  follower_leader_hostname: string;
  wake_word_enabled: boolean;
  wake_word_threshold: number;
  wake_word_arena_kib: number;
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
