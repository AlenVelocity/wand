// Shapes of everything that crosses between Rust and the web views.
// Mirrors src-tauri/src/{engine,config,gestures,joycon,app}.rs.

export type Pt = [number, number];

export interface OverlayCfg {
  prediction_ms: number;
  prediction_mouse_ms: number;
  confirm_ms: number;
  tail_fade_ms: number;
  tail_floor: number;
  width_min_px: number;
  width_max_px: number;
  speed_full_px_s: number;
  spark_rate: number;
}

export interface Monitor {
  x: number;
  y: number;
  w: number;
  h: number;
  scale: number;
  primary: boolean;
}

export type StrokeResult =
  | { kind: 'match'; gesture: string; label: string; icon: string; score: number; from: number[]; to: number[] }
  | { kind: 'no_match'; from: number[] }
  | { kind: 'captured'; from: number[] };

export type OverlayMsg =
  | { k: 'down'; id: number; t: number; x: number; y: number; lead_ms: number }
  | { k: 'move'; id: number; pts: number[] }
  | { k: 'up'; id: number; t: number; result: StrokeResult }
  | { k: 'cancel'; id: number };

export type Action =
  | { type: 'key_combo'; keys: string }
  | { type: 'media_key'; key: string; repeat: number }
  | { type: 'launch'; path: string; args: string }
  | { type: 'shell'; command: string }
  | { type: 'none' };

export interface Variant {
  ideal: Pt[];
  samples: Pt[][];
}

export interface Gesture {
  id: string;
  name: string;
  enabled: boolean;
  rotation: 'bounded' | 'free';
  action: Action;
  label: string;
  icon: string;
  variants: Variant[];
  builtin: boolean;
}

export interface Library {
  version: number;
  gestures: Gesture[];
}

export type LinkState = 'bluetooth_off' | 'searching' | 'connecting' | 'pairing' | 'connected' | 'reconnecting';

export interface DeviceStatus {
  state: LinkState;
  name: string;
  side: string;
  serial: string;
  battery_percent: number | null;
  battery_mv: number;
  charging: boolean;
  body_color: string;
  button_color: string;
  accent_color: string;
  has_magnetometer: boolean;
  has_mouse: boolean;
  interval_ms: number | null;
  report_hz: number;
  detail: string;
}

export interface Outcome {
  kind: 'match' | 'no_match' | 'too_short' | 'too_long' | 'captured';
  gesture_id: string | null;
  gesture: string | null;
  score: number;
  runner_up: number;
  closest: string | null;
  action: string | null;
  source: 'joycon' | 'mouse' | 'replay';
  length_px: number;
  duration_ms: number;
  points: number;
}

/** The optical sensor in the rail, as the engine reads it. */
export interface Surface {
  /** The buttons are mouse buttons right now. */
  active: boolean;
  /** The cursor is following the sensor right now. */
  tracking: boolean;
  /** The sensor alone says there is a surface, whichever way the rail faces. */
  seen: boolean;
  distance: number;
  roughness: number;
  /** How far the rail is from facing straight down, in degrees. */
  tilt_deg: number;
}

export interface Pose {
  q: [number, number, number, number];
  stationary: boolean;
  mag_in_use: boolean;
  report_hz: number;
  gyro_bias_dps: [number, number, number];
  surface: Surface;
}

export interface Config {
  recognizer: { min_score: number; min_margin: number; resample_points: number; rotation_tolerance_deg: number; scale_tolerance: number };
  stroke: { min_length_px: number; min_duration_ms: number; max_duration_ms: number; triggers: string[] };
  pointing: {
    px_per_degree: number;
    anchor: string;
    edge_margin_px: number;
    forward_axis: string;
    up_axis: string;
    invert_x: boolean;
    invert_y: boolean;
    recenter_buttons: string[];
    smoothing_min_cutoff_hz: number;
    smoothing_beta: number;
  };
  mouse: {
    enabled: boolean;
    sensitivity: number;
    smoothing_ms: number;
    scroll_speed: number;
    max_distance: number;
    max_roughness: number;
    max_tilt_deg: number;
  };
  fusion: { use_magnetometer: boolean };
  overlay: OverlayCfg;
  rumble: { enabled: boolean };
  dev: { enabled: boolean; hold_key: string; dry_run: boolean };
  actions: { shell: string };
}

export type Mode = 'normal' | 'recording' | 'paused';

export interface Snapshot {
  version: string;
  config: Config;
  config_error: string | null;
  library_error: string | null;
  config_path: string;
  data_dir: string;
  library: Library;
  status: DeviceStatus;
  mode: Mode;
  autostart: boolean;
  dry_run: boolean;
  imu_recording: string | null;
  replaying: boolean;
  history: Outcome[];
  calibration: { gyro_bias_dps: [number, number, number]; mag_valid: boolean; mag_field_ut: number };
  media_keys: [string, string][];
  monitors: Monitor[];
  /** macOS only: whether the app may press keys and move the cursor. */
  accessibility: boolean | null;
}

export interface RecordingCheck {
  consistency: number;
  pair_scores: number[];
  conflict: { gesture_id: string; gesture_name: string; score: number } | null;
}

export interface RecordingInfo {
  path: string;
  name: string;
  source: string;
  note: string;
  seconds: number;
  reports: number;
}

export const DEFAULT_OVERLAY_CFG: OverlayCfg = {
  prediction_ms: 10,
  prediction_mouse_ms: 4,
  confirm_ms: 400,
  tail_fade_ms: 850,
  tail_floor: 0.28,
  width_min_px: 1.3,
  width_max_px: 4.6,
  speed_full_px_s: 3600,
  spark_rate: 1,
};
