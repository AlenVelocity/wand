//! Joy-Con 2 wire format: advertisements, the command channel, input report
//! 0x05 and HD Rumble packets.
//!
//! Ported from Nadeflore/switch2-controllers and CareyScott/switch2controllerpc
//! (Python, bleak) and checked against ndeadly/switch2_controller_research,
//! which documents the byte layouts. Nothing here touches Bluetooth; `joycon.rs`
//! does the I/O. That keeps the format testable and lets the replay tool feed
//! recorded reports through the exact parser the live controller uses.

use uuid::Uuid;

/// Bluetooth SIG company id in the advertisement's manufacturer data.
pub const NINTENDO_COMPANY_ID: u16 = 0x0553;
pub const NINTENDO_VENDOR_ID: u16 = 0x057E;

pub const PID_JOYCON_R: u16 = 0x2066;
pub const PID_JOYCON_L: u16 = 0x2067;
pub const PID_PRO: u16 = 0x2069;
pub const PID_GAMECUBE: u16 = 0x2073;

/// Input report 0x05 (all controllers), notify.
pub const CHAR_INPUT: Uuid = Uuid::from_u128(0xab7de9be_89fe_49ad_828f_118f09df7fd2);
/// Command request, write.
pub const CHAR_COMMAND: Uuid = Uuid::from_u128(0x649d4ac9_8eb7_4e6c_af44_1ea54fe5f005);
/// Command response, notify.
pub const CHAR_RESPONSE: Uuid = Uuid::from_u128(0xc765a961_d9d8_4d36_a20a_5315b111836a);
/// Output report 0x01 (HD Rumble), write without response. One UUID per side.
pub const CHAR_RUMBLE_R: Uuid = Uuid::from_u128(0xfa19b0fb_cd1f_46a7_84a1_bbb09e00c149);
pub const CHAR_RUMBLE_L: Uuid = Uuid::from_u128(0x289326cb_a471_485d_a8f4_240c14f18241);
pub const CHAR_RUMBLE_PRO: Uuid = Uuid::from_u128(0xcc483f51_9258_427d_a939_630c31f72b05);

pub const CMD_MEMORY: u8 = 0x02;
pub const SUB_MEMORY_READ: u8 = 0x04;
pub const CMD_LEDS: u8 = 0x09;
pub const SUB_LEDS_PATTERN: u8 = 0x07;
pub const CMD_VIBRATION: u8 = 0x0A;
pub const SUB_VIBRATION_SAMPLE: u8 = 0x02;
pub const CMD_FEATURE: u8 = 0x0C;
pub const SUB_FEATURE_SET_MASK: u8 = 0x02;
pub const SUB_FEATURE_ENABLE: u8 = 0x04;
pub const CMD_PAIR: u8 = 0x15;
pub const SUB_PAIR_ADDRESSES: u8 = 0x01;
pub const SUB_PAIR_CONFIRM: u8 = 0x02;
pub const SUB_PAIR_FINISH: u8 = 0x03;
pub const SUB_PAIR_KEYS: u8 = 0x04;

pub const FEATURE_BUTTONS: u8 = 0x01;
pub const FEATURE_STICKS: u8 = 0x02;
pub const FEATURE_IMU: u8 = 0x04;
pub const FEATURE_MOUSE: u8 = 0x10;
pub const FEATURE_RUMBLE: u8 = 0x20;
pub const FEATURE_MAGNETOMETER: u8 = 0x80;

pub const ADDR_CONTROLLER_INFO: u32 = 0x0001_3000;

/// ICM-42670-P at its default full-scale ranges, AK09919 magnetometer.
pub const ACCEL_G_PER_LSB: f32 = 8.0 / 32767.0;
pub const GYRO_DPS_PER_LSB: f32 = 2000.0 / 32767.0;
pub const MAG_UT_PER_LSB: f32 = 0.15;

pub const INPUT_REPORT_LEN: usize = 63;

/// Button bits of the little-endian u32 at offset 4 of input report 0x05.
pub mod btn {
    pub const Y: u32 = 0x0000_0001;
    pub const X: u32 = 0x0000_0002;
    pub const B: u32 = 0x0000_0004;
    pub const A: u32 = 0x0000_0008;
    pub const SR_R: u32 = 0x0000_0010;
    pub const SL_R: u32 = 0x0000_0020;
    pub const R: u32 = 0x0000_0040;
    pub const ZR: u32 = 0x0000_0080;
    pub const MINUS: u32 = 0x0000_0100;
    pub const PLUS: u32 = 0x0000_0200;
    pub const R_STICK: u32 = 0x0000_0400;
    pub const L_STICK: u32 = 0x0000_0800;
    pub const HOME: u32 = 0x0000_1000;
    pub const CAPTURE: u32 = 0x0000_2000;
    pub const C: u32 = 0x0000_4000;
    pub const DOWN: u32 = 0x0001_0000;
    pub const UP: u32 = 0x0002_0000;
    pub const RIGHT: u32 = 0x0004_0000;
    pub const LEFT: u32 = 0x0008_0000;
    pub const SR_L: u32 = 0x0010_0000;
    pub const SL_L: u32 = 0x0020_0000;
    pub const L: u32 = 0x0040_0000;
    pub const ZL: u32 = 0x0080_0000;
    pub const GR: u32 = 0x0100_0000;
    pub const GL: u32 = 0x0200_0000;

    pub const NAMES: &[(&str, u32)] = &[
        ("Y", Y), ("X", X), ("B", B), ("A", A), ("SR_R", SR_R), ("SL_R", SL_R), ("R", R),
        ("ZR", ZR), ("MINUS", MINUS), ("PLUS", PLUS), ("R_STICK", R_STICK), ("L_STICK", L_STICK),
        ("HOME", HOME), ("CAPTURE", CAPTURE), ("C", C), ("DOWN", DOWN), ("UP", UP),
        ("RIGHT", RIGHT), ("LEFT", LEFT), ("SR_L", SR_L), ("SL_L", SL_L), ("L", L), ("ZL", ZL),
        ("GR", GR), ("GL", GL),
    ];

    pub fn from_name(name: &str) -> Option<u32> {
        let n = name.trim().to_ascii_uppercase();
        NAMES.iter().find(|(k, _)| *k == n).map(|(_, v)| *v)
    }

    /// OR together the named buttons, skipping names that do not exist.
    pub fn mask(names: &[String]) -> u32 {
        names.iter().filter_map(|n| from_name(n)).fold(0, |a, b| a | b)
    }
}

pub fn controller_name(pid: u16) -> &'static str {
    match pid {
        PID_JOYCON_R => "Joy-Con 2 (R)",
        PID_JOYCON_L => "Joy-Con 2 (L)",
        PID_PRO => "Pro Controller 2",
        PID_GAMECUBE => "GameCube Controller",
        _ => "Unknown controller",
    }
}

pub fn rumble_char(pid: u16) -> Option<Uuid> {
    match pid {
        PID_JOYCON_R => Some(CHAR_RUMBLE_R),
        PID_JOYCON_L => Some(CHAR_RUMBLE_L),
        PID_PRO => Some(CHAR_RUMBLE_PRO),
        _ => None,
    }
}

// ---------------------------------------------------------------- advertising

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Advert {
    pub vendor_id: u16,
    pub product_id: u16,
    /// Address of the host the controller wants to reconnect to, as a 48-bit
    /// integer. Zero while the controller is in pairing mode.
    pub host_address: u64,
    pub wake: bool,
}

/// Parse the manufacturer data that follows the 2-byte company id.
///
/// On air: `53 05 | 01 00 03 7e 05 PP PP 00 01 WW AA AA AA AA AA AA 0f ...`
/// with the company id first; BLE stacks strip it, so offsets here are 2 lower
/// than in the research notes.
pub fn parse_advert(data: &[u8]) -> Option<Advert> {
    if data.len() < 16 {
        return None;
    }
    let vendor_id = u16::from_le_bytes([data[3], data[4]]);
    let product_id = u16::from_le_bytes([data[5], data[6]]);
    if vendor_id != NINTENDO_VENDOR_ID {
        return None;
    }
    let mut addr = [0u8; 8];
    addr[..6].copy_from_slice(&data[10..16]);
    Some(Advert {
        vendor_id,
        product_id,
        host_address: u64::from_le_bytes(addr),
        wake: data[9] == 0x81,
    })
}

pub fn is_supported(pid: u16) -> bool {
    matches!(pid, PID_JOYCON_R | PID_JOYCON_L | PID_PRO)
}

// ------------------------------------------------------------------- commands

/// `[cmd, 0x91 request, 0x01 bluetooth, sub, 0, len, 0, 0, data...]`
pub fn command(cmd: u8, sub: u8, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + data.len());
    out.extend_from_slice(&[cmd, 0x91, 0x01, sub, 0x00, data.len() as u8, 0x00, 0x00]);
    out.extend_from_slice(data);
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub cmd: u8,
    pub sub: u8,
    pub status: u8,
    pub data: Vec<u8>,
}

/// `[cmd, 0x01 response, transport, sub, ?, status, 0, 0, data...]`
pub fn parse_response(buf: &[u8]) -> Option<Response> {
    if buf.len() < 8 || buf[1] != 0x01 {
        return None;
    }
    Some(Response { cmd: buf[0], sub: buf[3], status: buf[5], data: buf[8..].to_vec() })
}

pub fn cmd_read_memory(len: u8, address: u32) -> Vec<u8> {
    let mut d = vec![len, 0x7e, 0x00, 0x00];
    d.extend_from_slice(&address.to_le_bytes());
    command(CMD_MEMORY, SUB_MEMORY_READ, &d)
}

/// The payload of a memory read response: `[len, 7e, 0, 0, addr(4), bytes...]`.
pub fn memory_payload(data: &[u8], len: u8, address: u32) -> Option<&[u8]> {
    if data.len() < 8 || data[0] != len {
        return None;
    }
    if u32::from_le_bytes([data[4], data[5], data[6], data[7]]) != address {
        return None;
    }
    Some(&data[8..])
}

pub fn cmd_set_feature_mask(flags: u8) -> Vec<u8> {
    command(CMD_FEATURE, SUB_FEATURE_SET_MASK, &[flags, 0, 0, 0])
}

pub fn cmd_enable_features(flags: u8) -> Vec<u8> {
    command(CMD_FEATURE, SUB_FEATURE_ENABLE, &[flags, 0, 0, 0])
}

/// Player LED bit pattern (bit 0 = LED 1).
pub fn cmd_set_leds(pattern: u8) -> Vec<u8> {
    command(CMD_LEDS, SUB_LEDS_PATTERN, &[pattern, 0, 0, 0, 0, 0, 0, 0])
}

pub fn cmd_vibration_sample(id: u8) -> Vec<u8> {
    command(CMD_VIBRATION, SUB_VIBRATION_SAMPLE, &[id, 0, 0, 0])
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ControllerInfo {
    pub serial: String,
    pub vendor_id: u16,
    pub product_id: u16,
    /// Body and button colours, RGB.
    pub body_color: [u8; 3],
    pub button_color: [u8; 3],
}

/// Decode the 0x40 bytes at `ADDR_CONTROLLER_INFO`.
pub fn parse_controller_info(d: &[u8]) -> Option<ControllerInfo> {
    if d.len() < 0x25 {
        return None;
    }
    let serial = d[2..16].iter().filter(|b| b.is_ascii_graphic()).map(|&b| b as char).collect();
    Some(ControllerInfo {
        serial,
        vendor_id: u16::from_le_bytes([d[18], d[19]]),
        product_id: u16::from_le_bytes([d[20], d[21]]),
        body_color: [d[25], d[26], d[27]],
        button_color: [d[28], d[29], d[30]],
    })
}

// -------------------------------------------------------------------- pairing
//
// Nintendo replaces SMP with its own exchange over the command channel:
//   0x15/0x01  host sends its address (twice), controller answers with its own
//   0x15/0x04  host sends public key A1, controller answers B1; LTK = A1 ^ B1
//   0x15/0x02  host sends challenge A2, controller answers AES-128(LTK, A2)
//   0x15/0x03  controller commits the host address and LTK to flash
// After that the controller advertises with the host address in its
// manufacturer data whenever a button is pressed, which is how reconnection
// finds it. Windows never encrypts the link with the LTK, and the controller
// does not require it.

pub fn cmd_pair_addresses(host_address: u64) -> Vec<u8> {
    let a = host_address.to_le_bytes();
    let mut d = vec![0x00, 0x02];
    d.extend_from_slice(&a[..6]);
    d.extend_from_slice(&a[..6]);
    command(CMD_PAIR, SUB_PAIR_ADDRESSES, &d)
}

pub fn cmd_pair_keys(a1: &[u8; 16]) -> Vec<u8> {
    let mut d = vec![0x00];
    d.extend_from_slice(a1);
    command(CMD_PAIR, SUB_PAIR_KEYS, &d)
}

pub fn cmd_pair_confirm(a2: &[u8; 16]) -> Vec<u8> {
    let mut d = vec![0x00];
    d.extend_from_slice(a2);
    command(CMD_PAIR, SUB_PAIR_CONFIRM, &d)
}

pub fn cmd_pair_finish() -> Vec<u8> {
    command(CMD_PAIR, SUB_PAIR_FINISH, &[0x00])
}

/// What the controller should answer to challenge `a2`, in wire byte order:
/// AES-128 of the byte-reversed challenge under the byte-reversed LTK, sent as
/// it comes out of the cipher. Pinned by a captured exchange in the tests.
pub fn expected_confirm(a1: &[u8; 16], b1: &[u8; 16], a2: &[u8; 16]) -> [u8; 16] {
    use aes::cipher::{generic_array::GenericArray, BlockEncrypt, KeyInit};
    let mut ltk = [0u8; 16];
    for i in 0..16 {
        ltk[i] = a1[i] ^ b1[i];
    }
    ltk.reverse();
    let mut block = *a2;
    block.reverse();
    let cipher = aes::Aes128::new(GenericArray::from_slice(&ltk));
    let mut b = GenericArray::clone_from_slice(&block);
    cipher.encrypt_block(&mut b);
    let mut out = [0u8; 16];
    out.copy_from_slice(&b);
    out
}

// --------------------------------------------------------------- input report

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputReport {
    pub counter: u32,
    pub buttons: u32,
    pub left_stick: (u16, u16),
    pub right_stick: (u16, u16),
    pub mouse: [u16; 4],
    pub mag: [i16; 3],
    pub battery_mv: u16,
    pub charge_state: u8,
    pub battery_current: u16,
    pub motion_time_us: u32,
    pub temperature: i16,
    pub accel: [i16; 3],
    pub gyro: [i16; 3],
}

fn u16_at(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}
fn i16_at(d: &[u8], o: usize) -> i16 {
    i16::from_le_bytes([d[o], d[o + 1]])
}
fn u32_at(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}
fn stick_at(d: &[u8], o: usize) -> (u16, u16) {
    let v = d[o] as u32 | (d[o + 1] as u32) << 8 | (d[o + 2] as u32) << 16;
    ((v & 0xFFF) as u16, (v >> 12) as u16)
}

impl InputReport {
    /// Offsets (hex): 00 counter, 04 buttons, 0A/0D sticks, 10 mouse, 19 mag,
    /// 1F battery mV, 21 charge state, 22 current, 2A motion {time, temp,
    /// accel xyz, gyro xyz}.
    pub fn parse(d: &[u8]) -> Option<InputReport> {
        if d.len() < 0x3C {
            return None;
        }
        Some(InputReport {
            counter: u32_at(d, 0x00),
            buttons: u32_at(d, 0x04),
            left_stick: stick_at(d, 0x0A),
            right_stick: stick_at(d, 0x0D),
            mouse: [u16_at(d, 0x10), u16_at(d, 0x12), u16_at(d, 0x14), u16_at(d, 0x16)],
            mag: [i16_at(d, 0x19), i16_at(d, 0x1B), i16_at(d, 0x1D)],
            battery_mv: u16_at(d, 0x1F),
            charge_state: d[0x21],
            battery_current: u16_at(d, 0x22),
            motion_time_us: u32_at(d, 0x2A),
            temperature: i16_at(d, 0x2E),
            accel: [i16_at(d, 0x30), i16_at(d, 0x32), i16_at(d, 0x34)],
            gyro: [i16_at(d, 0x36), i16_at(d, 0x38), i16_at(d, 0x3A)],
        })
    }

    /// Inverse of `parse`, for the synthetic generator and round-trip tests.
    pub fn encode(&self) -> [u8; INPUT_REPORT_LEN] {
        let mut d = [0u8; INPUT_REPORT_LEN];
        d[0x00..0x04].copy_from_slice(&self.counter.to_le_bytes());
        d[0x04..0x08].copy_from_slice(&self.buttons.to_le_bytes());
        let put_stick = |d: &mut [u8], o: usize, s: (u16, u16)| {
            let v = (s.0 as u32 & 0xFFF) | (s.1 as u32 & 0xFFF) << 12;
            d[o] = v as u8;
            d[o + 1] = (v >> 8) as u8;
            d[o + 2] = (v >> 16) as u8;
        };
        put_stick(&mut d, 0x0A, self.left_stick);
        put_stick(&mut d, 0x0D, self.right_stick);
        for (i, m) in self.mouse.iter().enumerate() {
            d[0x10 + i * 2..0x12 + i * 2].copy_from_slice(&m.to_le_bytes());
        }
        for (i, m) in self.mag.iter().enumerate() {
            d[0x19 + i * 2..0x1B + i * 2].copy_from_slice(&m.to_le_bytes());
        }
        d[0x1F..0x21].copy_from_slice(&self.battery_mv.to_le_bytes());
        d[0x21] = self.charge_state;
        d[0x22..0x24].copy_from_slice(&self.battery_current.to_le_bytes());
        d[0x29] = 0x01;
        d[0x2A..0x2E].copy_from_slice(&self.motion_time_us.to_le_bytes());
        d[0x2E..0x30].copy_from_slice(&self.temperature.to_le_bytes());
        for (i, a) in self.accel.iter().enumerate() {
            d[0x30 + i * 2..0x32 + i * 2].copy_from_slice(&a.to_le_bytes());
        }
        for (i, g) in self.gyro.iter().enumerate() {
            d[0x36 + i * 2..0x38 + i * 2].copy_from_slice(&g.to_le_bytes());
        }
        d
    }

    pub fn accel_g(&self) -> [f32; 3] {
        self.accel.map(|v| v as f32 * ACCEL_G_PER_LSB)
    }
    pub fn gyro_rad_s(&self) -> [f32; 3] {
        self.gyro.map(|v| (v as f32 * GYRO_DPS_PER_LSB).to_radians())
    }
    /// `None` when the magnetometer feature is off (the field is all zero).
    pub fn mag_ut(&self) -> Option<[f32; 3]> {
        if self.mag == [0, 0, 0] {
            None
        } else {
            Some(self.mag.map(|v| v as f32 * MAG_UT_PER_LSB))
        }
    }
    /// 0x00 on battery; rises while a charger is attached; 0x20 once full.
    pub fn charging(&self) -> bool {
        self.charge_state != 0 && self.charge_state != 0x20
    }
}

/// Rough state of charge from cell voltage. The controller reports millivolts,
/// not a percentage, so this is a standard single-cell Li-ion curve and will be
/// a few points off under load.
pub fn battery_percent(mv: u16) -> u8 {
    const CURVE: &[(f32, f32)] = &[
        (3300.0, 0.0), (3500.0, 4.0), (3600.0, 10.0), (3700.0, 26.0), (3750.0, 40.0),
        (3800.0, 53.0), (3900.0, 71.0), (4000.0, 85.0), (4100.0, 95.0), (4180.0, 100.0),
    ];
    let v = mv as f32;
    if v <= CURVE[0].0 {
        return 0;
    }
    for w in CURVE.windows(2) {
        if v <= w[1].0 {
            let t = (v - w[0].0) / (w[1].0 - w[0].0);
            return (w[0].1 + t * (w[1].1 - w[0].1)).round() as u8;
        }
    }
    100
}

// --------------------------------------------------------------------- rumble

/// One 5 ms slice for the linear resonant actuator: a low and a high band,
/// each with a 9-bit frequency code and a 10-bit amplitude.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RumbleSample {
    pub lf_freq: u16,
    pub lf_amp: u16,
    pub hf_freq: u16,
    pub hf_amp: u16,
}

impl RumbleSample {
    /// The controller's idle values: default frequencies, zero amplitude.
    pub const SILENT: RumbleSample = RumbleSample { lf_freq: 0x0E1, lf_amp: 0, hf_freq: 0x1E1, hf_amp: 0 };

    /// A high-band pulse. `strength` is 0..1.
    pub fn tick(strength: f32) -> RumbleSample {
        let s = strength.clamp(0.0, 1.0);
        RumbleSample {
            lf_freq: 0x0E1,
            lf_amp: (s * 60.0) as u16,
            hf_freq: 0x1E2,
            hf_amp: (s * 960.0) as u16,
        }
    }

    /// A low-band pulse. `strength` is 0..1.
    pub fn thud(strength: f32) -> RumbleSample {
        let s = strength.clamp(0.0, 1.0);
        RumbleSample {
            lf_freq: 0x060,
            lf_amp: (s * 1000.0) as u16,
            hf_freq: 0x0C0,
            hf_amp: (s * 420.0) as u16,
        }
    }

    fn pack(&self) -> [u8; 5] {
        let v: u64 = (self.lf_freq as u64 & 0x1FF)
            | (self.lf_amp as u64 & 0x3FF) << 10
            | (self.hf_freq as u64 & 0x1FF) << 20
            | (self.hf_amp as u64 & 0x3FF) << 30;
        let b = v.to_le_bytes();
        [b[0], b[1], b[2], b[3], b[4]]
    }
}

/// Output report 0x01 for a Joy-Con 2: report id 0, a `0x5N` sequence byte,
/// then three samples.
pub fn rumble_packet(seq: u8, samples: [RumbleSample; 3]) -> [u8; 17] {
    let mut out = [0u8; 17];
    out[1] = 0x50 | (seq & 0x0F);
    for (i, s) in samples.iter().enumerate() {
        out[2 + i * 5..7 + i * 5].copy_from_slice(&s.pack());
    }
    out
}

/// The Pro Controller 2 has two actuators and takes the 16 bytes twice.
pub fn rumble_packet_pro(seq: u8, samples: [RumbleSample; 3]) -> [u8; 33] {
    let one = rumble_packet(seq, samples);
    let mut out = [0u8; 33];
    out[..17].copy_from_slice(&one);
    out[17..33].copy_from_slice(&one[1..17]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
    }

    #[test]
    fn advert_reconnect_and_pairing() {
        // From the research notes, minus the leading company id `53 05`.
        let standard = hex("01 00 03 7e 05 66 20 00 01 00 00 00 00 00 00 00 0f 00 00 00 00 00 00 00");
        let a = parse_advert(&standard).unwrap();
        assert_eq!(a.product_id, PID_JOYCON_R);
        assert_eq!(a.host_address, 0);
        assert!(!a.wake);

        let reconnect = hex("01 00 03 7e 05 69 20 00 01 00 5f 11 85 eb f1 48 0f 00 00 00 00 00 00 00");
        let a = parse_advert(&reconnect).unwrap();
        assert_eq!(a.product_id, PID_PRO);
        assert_eq!(a.host_address, 0x48f1_eb85_115f);

        let wake = hex("01 00 03 7e 05 69 20 00 01 81 5f 11 85 eb f1 48 0f 00 00 00 00 00 00 00");
        assert!(parse_advert(&wake).unwrap().wake);
        assert!(parse_advert(&hex("01 00 03 4c 00 69 20 00 01 00 00 00 00 00 00 00")).is_none());
    }

    #[test]
    fn commands_match_captured_traffic() {
        assert_eq!(cmd_read_memory(0x40, 0x13000), hex("02 91 01 04 00 08 00 00 40 7e 00 00 00 30 01 00"));
        assert_eq!(cmd_set_feature_mask(0x37), hex("0c 91 01 02 00 04 00 00 37 00 00 00"));
        assert_eq!(cmd_enable_features(0x37), hex("0c 91 01 04 00 04 00 00 37 00 00 00"));
        assert_eq!(cmd_set_leds(1), hex("09 91 01 07 00 08 00 00 01 00 00 00 00 00 00 00"));
        assert_eq!(cmd_vibration_sample(3), hex("0a 91 01 02 00 04 00 00 03 00 00 00"));
        assert_eq!(
            cmd_pair_addresses(0x48f1_eb85_115f),
            hex("15 91 01 01 00 0e 00 00 00 02 5f 11 85 eb f1 48 5f 11 85 eb f1 48")
        );
        assert_eq!(cmd_pair_finish(), hex("15 91 01 03 00 01 00 00 00"));
    }

    #[test]
    fn response_and_memory_payload() {
        let mut buf = hex("02 01 01 04 10 78 00 00 40 7e 00 00 00 30 01 00");
        buf.extend(std::iter::repeat(0xAB).take(0x40));
        let r = parse_response(&buf).unwrap();
        assert_eq!((r.cmd, r.sub, r.status), (0x02, 0x04, 0x78));
        assert_eq!(memory_payload(&r.data, 0x40, 0x13000).unwrap().len(), 0x40);
        assert!(memory_payload(&r.data, 0x40, 0x13080).is_none());
        assert!(parse_response(&hex("02 91 01 04 00 08 00 00")).is_none());
    }

    #[test]
    fn pairing_confirm_matches_captured_exchange() {
        // Key exchange and challenge/response captured from a real console
        // pairing, as documented in the research notes.
        let a1: [u8; 16] = hex("35 03 e9 29 82 87 71 24 be a8 0c 66 46 15 83 4b").try_into().unwrap();
        let b1: [u8; 16] = hex("5c f6 ee 79 2c df 05 e1 ba 2b 63 25 c4 1a 5f 10").try_into().unwrap();
        let a2: [u8; 16] = hex("6f c6 df 8a d8 fe df 15 bb 8c 15 e9 1f 32 05 44").try_into().unwrap();
        let b2 = hex("13 4c 97 f5 11 b9 b6 dd 4d 86 fd 40 f5 36 e9 ed");
        assert_eq!(expected_confirm(&a1, &b1, &a2).to_vec(), b2);
    }

    #[test]
    fn input_report_round_trip() {
        let r = InputReport {
            counter: 0x0102_0304,
            buttons: btn::ZR | btn::A | btn::ZL | btn::GL,
            left_stick: (0x123, 0xABC),
            right_stick: (0xFFF, 0x001),
            mouse: [1, 2, 3, 4],
            mag: [-120, 340, -2000],
            battery_mv: 3749,
            charge_state: 0x34,
            battery_current: 77,
            motion_time_us: 123_456_789,
            temperature: -40,
            accel: [12, -4096, 4095],
            gyro: [-32768, 32767, 5],
        };
        let bytes = r.encode();
        assert_eq!(bytes.len(), 63);
        assert_eq!(InputReport::parse(&bytes).unwrap(), r);
        // Button bytes as the research notes lay them out.
        assert_eq!(bytes[4] & 0x80, 0x80, "ZR is bit 7 of button byte 0");
        assert_eq!(bytes[6] & 0x80, 0x80, "ZL is bit 7 of button byte 2");
        assert_eq!(bytes[7] & 0x02, 0x02, "GL is bit 1 of button byte 3");
        assert!(InputReport::parse(&bytes[..40]).is_none());
    }

    #[test]
    fn units() {
        let r = InputReport { accel: [0, 0, 4096], gyro: [0, 16384, 0], mag: [100, 0, 0], ..Default::default() };
        assert!((r.accel_g()[2] - 1.0).abs() < 0.01);
        assert!((r.gyro_rad_s()[1].to_degrees() - 1000.0).abs() < 1.0);
        assert!((r.mag_ut().unwrap()[0] - 15.0).abs() < 1e-4);
        assert!(InputReport::default().mag_ut().is_none());
    }

    #[test]
    fn battery_curve_is_monotonic() {
        let mut last = 0;
        for mv in (3200..4300).step_by(10) {
            let p = battery_percent(mv);
            assert!(p >= last);
            last = p;
        }
        assert_eq!(battery_percent(3000), 0);
        assert_eq!(battery_percent(4200), 100);
        assert!((35..=45).contains(&battery_percent(3749)));
    }

    #[test]
    fn rumble_packing_matches_reference() {
        // Reference: value = lf_freq | lf_amp << 10 | hf_freq << 20 | hf_amp << 30, 5 bytes LE.
        let s = RumbleSample { lf_freq: 0x060, lf_amp: 0x350, hf_freq: 0x0C0, hf_amp: 0x250 };
        let v: u64 = 0x060 | 0x350 << 10 | 0x0C0 << 20 | 0x250 << 30;
        assert_eq!(s.pack(), v.to_le_bytes()[..5]);
        let p = rumble_packet(0x13, [s, RumbleSample::SILENT, RumbleSample::SILENT]);
        assert_eq!(p[0], 0x00);
        assert_eq!(p[1], 0x53);
        assert_eq!(&p[2..7], &s.pack());
        assert_eq!(rumble_packet_pro(1, [s; 3]).len(), 33);
    }
}
