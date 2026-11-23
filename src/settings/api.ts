// Everything the settings window asks of the backend, in one place.
//
// Inside the app these are Tauri commands and events. In a plain browser
// (pnpm dev, for working on the interface) a small in-memory stand-in answers
// instead, so every page can be seen and clicked through without the app.

import type { Action, DeviceStatus, Gesture, Library, Mode, Outcome, Pose, Pt, RecordingCheck, RecordingInfo, Snapshot } from '../shared/types';
import { BUILTINS } from '../shared/gestures';

export const inApp = '__TAURI_INTERNALS__' in window;

type Unlisten = () => void;

export interface Backend {
  snapshot(): Promise<Snapshot>;
  setMode(mode: Mode): Promise<void>;
  saveGesture(gesture: Gesture): Promise<Library>;
  createGesture(args: { name: string; strokes: Pt[][]; freeRotation: boolean; action: Action; label: string; icon: string; replaceId?: string }): Promise<Library>;
  deleteGesture(id: string): Promise<Library>;
  resetGestures(): Promise<Library>;
  checkRecordings(strokes: Pt[][], ignoreId?: string): Promise<RecordingCheck>;
  setConfigValue(section: string, key: string, value: boolean | number | string): Promise<void>;
  setAutostart(enabled: boolean): Promise<void>;
  openConfig(): Promise<void>;
  openDataDir(): Promise<void>;
  pickProgram(): Promise<string | null>;
  deviceAction(action: string): Promise<unknown>;
  magProgress(): Promise<{ coverage: number; samples: number } | null>;
  imuRecordStart(): Promise<string>;
  imuRecordStop(): Promise<string | null>;
  listRecordings(): Promise<RecordingInfo[]>;
  replayFile(path: string, runActions: boolean): Promise<void>;
  replayGesture(id: string): Promise<void>;
  replayStop(): Promise<void>;
  requestAccessibility(): Promise<boolean>;
  subscribePose(onPose: (pose: Pose) => void): Promise<Unlisten>;
  on<T>(event: string, handler: (payload: T) => void): Promise<Unlisten>;
}

async function tauriBackend(): Promise<Backend> {
  const { invoke, Channel } = await import('@tauri-apps/api/core');
  const { listen } = await import('@tauri-apps/api/event');
  return {
    snapshot: () => invoke('snapshot'),
    setMode: (mode) => invoke('set_mode', { mode }),
    saveGesture: (gesture) => invoke('save_gesture', { gesture }),
    createGesture: (a) => invoke('create_gesture', { name: a.name, strokes: a.strokes, freeRotation: a.freeRotation, action: a.action, label: a.label, icon: a.icon, replaceId: a.replaceId ?? null }),
    deleteGesture: (id) => invoke('delete_gesture', { id }),
    resetGestures: () => invoke('reset_gestures'),
    checkRecordings: (strokes, ignoreId) => invoke('check_recordings', { strokes, ignoreId: ignoreId ?? null }),
    setConfigValue: (section, key, value) => invoke('set_config_value', { section, key, value }),
    setAutostart: (enabled) => invoke('set_autostart', { enabled }),
    openConfig: () => invoke('open_config'),
    openDataDir: () => invoke('open_data_dir'),
    pickProgram: () => invoke('pick_program'),
    deviceAction: (action) => invoke('device_action', { action }),
    magProgress: () => invoke('mag_progress'),
    imuRecordStart: () => invoke('imu_record_start'),
    imuRecordStop: () => invoke('imu_record_stop'),
    listRecordings: () => invoke('list_recordings'),
    replayFile: (path, runActions) => invoke('replay_file', { path, runActions }),
    replayGesture: (id) => invoke('replay_gesture', { id }),
    replayStop: () => invoke('replay_stop'),
    requestAccessibility: () => invoke('request_accessibility'),
    subscribePose: async (onPose) => {
      const channel = new Channel<Pose>();
      channel.onmessage = onPose;
      await invoke('pose_subscribe', { channel });
      return () => void invoke('pose_unsubscribe');
    },
    on: async (event, handler) => listen(event, (e) => handler(e.payload as never)),
  };
}

// ---------------------------------------------------------------- browser stand-in

function mockLibrary(): Library {
  return {
    version: 3,
    gestures: BUILTINS.map(
      (b): Gesture => ({
        id: b.id,
        name: b.name,
        enabled: true,
        rotation: b.free ? 'free' : 'bounded',
        action: b.action,
        label: b.label,
        icon: b.icon,
        variants: b.variants.map((ideal) => ({ ideal, samples: [] })),
        builtin: true,
      }),
    ),
  };
}

function mockBackend(): Backend {
  const q = new URLSearchParams(location.search);
  const connected = q.get('device') !== 'none';
  let library = mockLibrary();
  let mode: Mode = 'normal';
  const handlers = new Map<string, Set<(p: unknown) => void>>();
  const emit = (event: string, payload: unknown) => handlers.get(event)?.forEach((h) => h(payload));
  // Lets a console (or a test) push events the way the app would.
  (window as unknown as { __wandEmit: typeof emit }).__wandEmit = emit;
  const status: DeviceStatus = connected
    ? {
        state: 'connected', name: q.get('side') === 'left' ? 'Joy-Con 2 (L)' : 'Joy-Con 2 (R)', side: q.get('side') === 'left' ? 'left' : 'right', serial: 'HEJ70012345678', battery_percent: 82, battery_mv: 3980,
        charging: false, body_color: '#2d2f33', button_color: '#1e1e1e', accent_color: q.get('side') === 'left' ? '#3ec1ea' : '#ff5a4d', has_magnetometer: true, has_mouse: true, interval_ms: 15, report_hz: 66, detail: 'Connected.',
      }
    : {
        state: 'searching', name: '', side: '', serial: '', battery_percent: null, battery_mv: 0, charging: false, body_color: '', button_color: '', accent_color: '',
        has_magnetometer: false, has_mouse: false, interval_ms: null, report_hz: 0,
        detail: "Hold the small sync button on the Joy-Con's rail until its lights chase. A controller already paired with this PC reconnects when you press any button.",
      };
  const config = {
    recognizer: { min_score: 0.8, min_margin: 0.04, resample_points: 64, rotation_tolerance_deg: 12, scale_tolerance: 0.2 },
    stroke: { min_length_px: 90, min_duration_ms: 60, max_duration_ms: 6000, triggers: ['ZR', 'ZL'] },
    pointing: { px_per_degree: 42, anchor: 'pointing', edge_margin_px: 120, forward_axis: '+y', up_axis: '+z', invert_x: false, invert_y: false, recenter_buttons: ['R_STICK', 'L_STICK'], smoothing_min_cutoff_hz: 9, smoothing_beta: 0.035 },
    mouse: { enabled: true, sensitivity: 1, smoothing_ms: 15, scroll_speed: 18, max_distance: 1000, max_roughness: 4000, max_tilt_deg: 50 },
    fusion: { use_magnetometer: true },
    overlay: { prediction_ms: 10, prediction_mouse_ms: 4, confirm_ms: 400, tail_fade_ms: 850, tail_floor: 0.28, width_min_px: 1.3, width_max_px: 4.6, speed_full_px_s: 3600, spark_rate: 1 },
    rumble: { enabled: true },
    dev: { enabled: q.get('dev') === '1', hold_key: 'RightCtrl', dry_run: false },
    actions: { shell: 'cmd' },
  };
  const history: Outcome[] = [
    { kind: 'match', gesture_id: 'circle', gesture: 'Circle', score: 0.94, runner_up: 0.31, closest: null, action: 'Win+PrintScreen', source: 'joycon', length_px: 1620, duration_ms: 640, points: 82 },
    { kind: 'no_match', gesture_id: null, gesture: null, score: 0.71, runner_up: 0.52, closest: 'Z', action: null, source: 'joycon', length_px: 900, duration_ms: 510, points: 60 },
    { kind: 'match', gesture_id: 'flick-up', gesture: 'Flick up', score: 0.98, runner_up: 0.4, closest: null, action: 'Volume up', source: 'mouse', length_px: 420, duration_ms: 180, points: 140 },
  ];
  let autostart = false;
  return {
    snapshot: async () => ({
      version: '0.2.0', config, config_error: null, library_error: null, config_path: 'C:\\Users\\you\\AppData\\Roaming\\app.wand.desktop\\wand.toml',
      data_dir: 'C:\\Users\\you\\AppData\\Roaming\\app.wand.desktop', library, status, mode, autostart, dry_run: false, imu_recording: null, replaying: false,
      history, calibration: { gyro_bias_dps: [0.42, -0.81, 0.17], mag_valid: q.get('mag') !== '0', mag_field_ut: 47.2 },
      media_keys: [['volume_up', 'Volume up'], ['volume_down', 'Volume down'], ['mute', 'Mute'], ['play_pause', 'Play / pause'], ['next_track', 'Next track'], ['prev_track', 'Previous track'], ['stop', 'Stop']],
      monitors: [],
      accessibility: q.get('access') === '0' ? false : null,
    }),
    setMode: async (m) => {
      mode = m;
      emit('wand://changed', null);
    },
    saveGesture: async (gesture) => {
      library = { ...library, gestures: library.gestures.map((g) => (g.id === gesture.id ? gesture : g)) };
      emit('wand://changed', null);
      return library;
    },
    createGesture: async (a) => {
      const ideal = a.strokes[0];
      library = {
        ...library,
        gestures: [...library.gestures, { id: a.name.toLowerCase().replace(/\W+/g, '-'), name: a.name, enabled: true, rotation: a.freeRotation ? 'free' : 'bounded', action: a.action, label: a.label, icon: a.icon, variants: [{ ideal, samples: a.strokes }], builtin: false }],
      };
      emit('wand://changed', null);
      return library;
    },
    deleteGesture: async (id) => {
      library = { ...library, gestures: library.gestures.filter((g) => g.id !== id) };
      emit('wand://changed', null);
      return library;
    },
    resetGestures: async () => {
      library = mockLibrary();
      emit('wand://changed', null);
      return library;
    },
    checkRecordings: async () => ({ consistency: 0.91, pair_scores: [0.93, 0.91, 0.94], conflict: null }),
    setConfigValue: async (section, key, value) => {
      (config as unknown as Record<string, Record<string, unknown>>)[section][key] = value;
      emit('wand://changed', null);
    },
    setAutostart: async (enabled) => {
      autostart = enabled;
      emit('wand://changed', null);
    },
    openConfig: async () => {},
    openDataDir: async () => {},
    pickProgram: async () => 'C:\\Program Files\\Example\\example.exe',
    deviceAction: async (action) => {
      if (action === 'calibrate_gyro') setTimeout(() => emit('wand://gyro-calibrated', null), 2000);
      return null;
    },
    magProgress: async () => ({ coverage: Math.min(1, (performance.now() % 9000) / 7000), samples: 900 }),
    imuRecordStart: async () => 'recordings\\session-1.wandrec',
    imuRecordStop: async () => 'recordings\\session-1.wandrec (2400 reports)',
    listRecordings: async () => [{ path: 'recordings/session-1.wandrec', name: 'session-1', source: 'joycon', note: 'Joy-Con 2 (R)', seconds: 19.2, reports: 2400 }],
    replayFile: async () => {},
    replayGesture: async () => {},
    replayStop: async () => {},
    requestAccessibility: async () => false,
    subscribePose: async (onPose) => {
      // A slow, wrist-like wobble so the 3D preview has something to show.
      const t0 = performance.now();
      const q0 = new URLSearchParams(location.search);
      const id = window.setInterval(() => {
        const t = (performance.now() - t0) / 1000;
        const yaw = 0.35 * Math.sin(t * 0.7);
        const pitch = 0.3 + 0.2 * Math.sin(t * 1.1);
        const roll = 0.25 * Math.sin(t * 0.9 + 1);
        const [cy, sy, cp, sp, cr, sr] = [Math.cos(yaw / 2), Math.sin(yaw / 2), Math.cos(pitch / 2), Math.sin(pitch / 2), Math.cos(roll / 2), Math.sin(roll / 2)];
        // q = Rz(yaw) * Rx(pitch) * Ry(roll)
        const ax = { x: sp, y: 0, z: 0, w: cp };
        const ay = { x: 0, y: sr, z: 0, w: cr };
        const az = { x: 0, y: 0, z: sy, w: cy };
        const mul = (a: typeof ax, b: typeof ax) => ({
          x: a.w * b.x + a.x * b.w + a.y * b.z - a.z * b.y,
          y: a.w * b.y - a.x * b.z + a.y * b.w + a.z * b.x,
          z: a.w * b.z + a.x * b.y - a.y * b.x + a.z * b.w,
          w: a.w * b.w - a.x * b.x - a.y * b.y - a.z * b.z,
        });
        const q = mul(az, mul(ax, ay));
        // Set down for four seconds, picked up for four.
        const down = q0.get('surface') === '1' || (q0.get('surface') !== '0' && t % 8 > 4);
        const surface = down
          ? { active: true, tracking: true, seen: true, distance: 312, roughness: 1180, tilt_deg: 3 }
          : { active: false, tracking: false, seen: false, distance: 2600, roughness: 5400, tilt_deg: 74 };
        onPose({ q: [q.x, q.y, q.z, q.w], stationary: false, mag_in_use: true, report_hz: 66, gyro_bias_dps: [0.42, -0.81, 0.17], surface });
      }, 16);
      return () => window.clearInterval(id);
    },
    on: async (event, handler) => {
      if (!handlers.has(event)) handlers.set(event, new Set());
      handlers.get(event)!.add(handler as (p: unknown) => void);
      return () => handlers.get(event)?.delete(handler as (p: unknown) => void);
    },
  };
}

let instance: Promise<Backend> | null = null;

export function backend(): Promise<Backend> {
  instance ??= inApp ? tauriBackend() : Promise.resolve(mockBackend());
  return instance;
}

/** What an action is, in a few words, for cards and lists. */
export function describeAction(action: Action, mediaKeys: [string, string][]): string {
  switch (action.type) {
    case 'key_combo':
      return action.keys;
    case 'media_key': {
      const name = mediaKeys.find(([k]) => k === action.key)?.[1] ?? action.key;
      return action.repeat > 1 ? `${name} ×${action.repeat}` : name;
    }
    case 'launch':
      return action.path.split(/[\\/]/).pop() ?? action.path;
    case 'shell':
      return action.command;
    default:
      return 'Nothing';
  }
}
