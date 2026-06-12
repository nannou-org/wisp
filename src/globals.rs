//! The wisp-provided globals uniform: recognized member names, layout packing and a
//! date helper.
//!
//! A wisp shader opts into per-frame globals by declaring a uniform struct at
//! `@group(0) @binding(0)` whose members are any subset of the recognized names
//! below. Wisp writes each member at its reflected offset, so declaring only what a
//! shader uses is fine.

use crate::schema::ParamType;
use bevy::math::{Vec2, Vec4};

/// A recognized member of the wisp globals uniform struct.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GlobalKind {
    /// `resolution: vec2<f32>` - size of the current pass target in physical pixels.
    Resolution,
    /// `time: f32` - seconds since the app started.
    Time,
    /// `time_delta: f32` - seconds elapsed since the previous frame.
    TimeDelta,
    /// `frame: u32` - frames rendered since the app started.
    Frame,
    /// `pass_index: u32` - index of the current pass, in declaration order.
    PassIndex,
    /// `mouse: vec4<f32>` - cursor position in physical pixels (xy, matching the
    /// fullscreen `uv` orientation), 1.0 while the primary button is held (z) and
    /// 1.0 on the frame it was pressed (w).
    Mouse,
    /// `date: vec4<f32>` - (year, month, day, seconds since midnight), UTC.
    Date,
}

/// The reflected layout of a shader's globals uniform struct.
#[derive(Clone, Debug, PartialEq)]
pub struct GlobalsSchema {
    /// Total size of the struct in bytes, including trailing padding.
    pub size: u32,
    /// The recognized members with their byte offsets.
    pub fields: Vec<(GlobalKind, u32)>,
}

/// The per-pass values written into a globals uniform.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GlobalsValues {
    pub resolution: Vec2,
    pub time: f32,
    pub time_delta: f32,
    pub frame: u32,
    pub pass_index: u32,
    pub mouse: Vec4,
    pub date: Vec4,
}

impl GlobalKind {
    pub const ALL: [Self; 7] = [
        Self::Resolution,
        Self::Time,
        Self::TimeDelta,
        Self::Frame,
        Self::PassIndex,
        Self::Mouse,
        Self::Date,
    ];

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Resolution => "resolution",
            Self::Time => "time",
            Self::TimeDelta => "time_delta",
            Self::Frame => "frame",
            Self::PassIndex => "pass_index",
            Self::Mouse => "mouse",
            Self::Date => "date",
        }
    }

    /// The WGSL type the member must be declared with.
    pub fn param_ty(self) -> ParamType {
        match self {
            Self::Resolution => ParamType::Vec2,
            Self::Time | Self::TimeDelta => ParamType::F32,
            Self::Frame | Self::PassIndex => ParamType::U32,
            Self::Mouse | Self::Date => ParamType::Vec4,
        }
    }
}

/// Pack the given values into a byte buffer matching the reflected layout.
pub fn pack_globals(schema: &GlobalsSchema, values: &GlobalsValues) -> Vec<u8> {
    let mut bytes = vec![0u8; schema.size as usize];
    for &(kind, offset) in &schema.fields {
        let offset = offset as usize;
        match kind {
            GlobalKind::Resolution => write(&mut bytes, offset, &values.resolution.to_array()),
            GlobalKind::Time => write(&mut bytes, offset, &[values.time]),
            GlobalKind::TimeDelta => write(&mut bytes, offset, &[values.time_delta]),
            GlobalKind::Frame => write(&mut bytes, offset, &[values.frame]),
            GlobalKind::PassIndex => write(&mut bytes, offset, &[values.pass_index]),
            GlobalKind::Mouse => write(&mut bytes, offset, &values.mouse.to_array()),
            GlobalKind::Date => write(&mut bytes, offset, &values.date.to_array()),
        }
    }
    bytes
}

/// `(year, month, day, seconds since midnight)` for the given seconds since the
/// UNIX epoch, UTC.
pub fn date_vec4(unix_secs: u64) -> [f32; 4] {
    let days = (unix_secs / 86_400) as i64;
    let secs = (unix_secs % 86_400) as f32;
    let (y, m, d) = civil_from_days(days);
    [y as f32, m as f32, d as f32, secs]
}

fn write<T: bytemuck::NoUninit>(bytes: &mut [u8], offset: usize, value: &[T]) {
    let src = bytemuck::cast_slice(value);
    bytes[offset..offset + src.len()].copy_from_slice(src);
}

// Howard Hinnant's `civil_from_days`: days since 1970-01-01 to (year, month, day).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch() {
        assert_eq!(date_vec4(0), [1970.0, 1.0, 1.0, 0.0]);
    }

    #[test]
    fn day_two() {
        assert_eq!(date_vec4(86_400), [1970.0, 1.0, 2.0, 0.0]);
    }

    #[test]
    fn known_dates() {
        // 2025-01-01 00:00:00 UTC.
        assert_eq!(date_vec4(1_735_689_600), [2025.0, 1.0, 1.0, 0.0]);
        // One hour, one minute and one second later.
        assert_eq!(
            date_vec4(1_735_689_600 + 3_661),
            [2025.0, 1.0, 1.0, 3_661.0]
        );
        // 2000-02-29 (leap day) is 11_016 days after the epoch.
        assert_eq!(date_vec4(11_016 * 86_400), [2000.0, 2.0, 29.0, 0.0]);
    }

    #[test]
    fn pack_offsets() {
        let schema = GlobalsSchema {
            size: 16,
            fields: vec![(GlobalKind::Time, 0), (GlobalKind::Resolution, 8)],
        };
        let values = GlobalsValues {
            time: 2.0,
            resolution: Vec2::new(640.0, 480.0),
            ..Default::default()
        };
        let bytes = pack_globals(&schema, &values);
        assert_eq!(bytes.len(), 16);
        let floats: &[f32] = bytemuck::cast_slice(&bytes);
        assert_eq!(floats, &[2.0, 0.0, 640.0, 480.0]);
    }

    #[test]
    fn pack_u32_fields() {
        let schema = GlobalsSchema {
            size: 8,
            fields: vec![(GlobalKind::Frame, 0), (GlobalKind::PassIndex, 4)],
        };
        let values = GlobalsValues {
            frame: 7,
            pass_index: 2,
            ..Default::default()
        };
        let bytes = pack_globals(&schema, &values);
        let words: &[u32] = bytemuck::cast_slice(&bytes);
        assert_eq!(words, &[7, 2]);
    }
}
