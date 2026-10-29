//! Deterministic rotation engine.
//!
//! Everything here is a pure function of a [`ScheduleSpec`] and an instant. Nothing reads the
//! clock, the database or the network, so "who is on call at T?" has the same answer after a
//! restart, on every worker, and whether or not a background job ran at the rotation boundary.
//!
//! Conventions:
//! * All intervals are half-open: a shift includes its start and excludes its end.
//! * `hours` rotations advance in absolute time. `days` and `weeks` rotations advance in the
//!   schedule's local calendar, so a 09:00 hand-over stays at 09:00 across daylight-saving changes.
//! * A local time skipped by a DST gap resolves to the first valid instant after the gap; an
//!   ambiguous local time (DST fold) resolves to its earlier occurrence.
//! * Rotation configuration is versioned. A version applies from its `effective_from` until the
//!   next version begins, which is how edits avoid rewriting history. Every version rotates from
//!   the schedule's start anchor, so participant order continues predictably.
//! * Overrides replace the rotation assignee for their window. When overrides overlap, the one
//!   created last wins.

use chrono::{DateTime, Duration, LocalResult, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RotationUnit {
    Hours,
    Days,
    Weeks,
}

impl RotationUnit {
    pub fn as_str(self) -> &'static str {
        match self {
            RotationUnit::Hours => "hours",
            RotationUnit::Days => "days",
            RotationUnit::Weeks => "weeks",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "hours" => Some(RotationUnit::Hours),
            "days" => Some(RotationUnit::Days),
            "weeks" => Some(RotationUnit::Weeks),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    Primary,
    Secondary,
}

impl Layer {
    pub const ALL: [Layer; 2] = [Layer::Primary, Layer::Secondary];

    pub fn as_str(self) -> &'static str {
        match self {
            Layer::Primary => "primary",
            Layer::Secondary => "secondary",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "primary" => Some(Layer::Primary),
            "secondary" => Some(Layer::Secondary),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RotationVersion {
    pub effective_from: DateTime<Utc>,
    pub interval: u32,
    pub unit: RotationUnit,
    pub primary: Vec<Uuid>,
    pub secondary: Vec<Uuid>,
}

impl RotationVersion {
    fn participants(&self, layer: Layer) -> &[Uuid] {
        match layer {
            Layer::Primary => &self.primary,
            Layer::Secondary => &self.secondary,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Override {
    pub id: Uuid,
    pub layer: Layer,
    pub user_id: Uuid,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct ScheduleSpec {
    pub timezone: Tz,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    /// Sorted by `effective_from` ascending. [`ScheduleSpec::new`] sorts them.
    pub versions: Vec<RotationVersion>,
    /// In creation order; later entries win where they overlap.
    pub overrides: Vec<Override>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ShiftSource {
    Rotation { index: i64 },
    Override { override_id: Uuid },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Shift {
    pub layer: Layer,
    pub user_id: Uuid,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub source: ShiftSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Transition {
    pub layer: Layer,
    pub at: DateTime<Utc>,
    pub previous_user_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
    /// The shift that begins at `at`, when someone is on call afterwards.
    pub shift: Option<Shift>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SpecError {
    #[error("the rotation interval must be at least 1")]
    ZeroInterval,
    #[error("the end must be after the start")]
    EndBeforeStart,
    #[error("the primary rotation needs at least one participant")]
    NoPrimaryParticipants,
    #[error("a participant appears twice in the {0} rotation")]
    DuplicateParticipant(&'static str),
    #[error("the schedule has no rotation configuration")]
    NoVersions,
}

/// The longest window the engine will expand in one call, as a guard against runaway requests.
pub const MAX_WINDOW_DAYS: i64 = 400;

impl ScheduleSpec {
    pub fn new(timezone: Tz, start: DateTime<Utc>, end: Option<DateTime<Utc>>, mut versions: Vec<RotationVersion>, overrides: Vec<Override>) -> Self {
        versions.sort_by_key(|v| v.effective_from);
        ScheduleSpec { timezone, start, end, versions, overrides }
    }

    pub fn validate(&self) -> Result<(), SpecError> {
        if let Some(end) = self.end {
            if end <= self.start {
                return Err(SpecError::EndBeforeStart);
            }
        }
        if self.versions.is_empty() {
            return Err(SpecError::NoVersions);
        }
        for version in &self.versions {
            validate_version(version)?;
        }
        Ok(())
    }

    /// The instant at which rotation shift `n` of `version` starts.
    pub fn boundary(&self, version: &RotationVersion, n: i64) -> DateTime<Utc> {
        let interval = i64::from(version.interval.max(1));
        match version.unit {
            RotationUnit::Hours => self.start + Duration::hours(interval * n),
            RotationUnit::Days | RotationUnit::Weeks => {
                let days_per_unit = if version.unit == RotationUnit::Weeks { 7 } else { 1 };
                let local_anchor = self.start.with_timezone(&self.timezone).naive_local();
                let local = local_anchor + Duration::days(interval * days_per_unit * n);
                resolve_local(self.timezone, local)
            }
        }
    }

    /// Index of the rotation shift of `version` that contains `t` (t must be >= start).
    fn index_at(&self, version: &RotationVersion, t: DateTime<Utc>) -> i64 {
        let interval = i64::from(version.interval.max(1));
        let nominal_seconds = match version.unit {
            RotationUnit::Hours => interval * 3600,
            RotationUnit::Days => interval * 86_400,
            RotationUnit::Weeks => interval * 7 * 86_400,
        };
        let mut n = ((t - self.start).num_seconds().max(0)) / nominal_seconds;
        // Local-calendar rotations drift from the nominal length by at most a DST offset, so the
        // estimate is off by one shift at most. Correct it against the real boundaries.
        while n > 0 && self.boundary(version, n) > t {
            n -= 1;
        }
        while self.boundary(version, n + 1) <= t {
            n += 1;
        }
        n
    }

    /// The window during which `self.versions[i]` is in force, before clipping to a query window.
    /// The first version always covers the schedule from its start, so moving the start of a
    /// schedule that has not begun yet never leaves a gap before the original start.
    fn version_range(&self, i: usize) -> (DateTime<Utc>, Option<DateTime<Utc>>) {
        let from = if i == 0 { self.start } else { self.versions[i].effective_from.max(self.start) };
        let next = self.versions.get(i + 1).map(|v| v.effective_from);
        let until = match (next, self.end) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        (from, until)
    }

    /// Rotation shifts (before overrides) for one layer that intersect `[from, to)`.
    /// Shifts are clipped to the version and schedule boundaries but not to the query window, so
    /// callers see each shift's real start and end.
    fn rotation_shifts(&self, layer: Layer, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<Shift> {
        let mut shifts = Vec::new();
        for (i, version) in self.versions.iter().enumerate() {
            let participants = version.participants(layer);
            if participants.is_empty() {
                continue;
            }
            let (range_start, range_end) = self.version_range(i);
            if range_end.is_some_and(|end| end <= range_start) {
                continue;
            }
            let window_start = range_start.max(from);
            let window_end = range_end.map_or(to, |end| end.min(to));
            if window_end <= window_start {
                continue;
            }
            let mut n = self.index_at(version, window_start);
            loop {
                let start = self.boundary(version, n).max(range_start);
                if start >= window_end {
                    break;
                }
                let natural_end = self.boundary(version, n + 1);
                let end = range_end.map_or(natural_end, |e| natural_end.min(e));
                let len = participants.len() as i64;
                let user_id = participants[n.rem_euclid(len) as usize];
                shifts.push(Shift { layer, user_id, start, end, source: ShiftSource::Rotation { index: n } });
                n += 1;
            }
        }
        shifts
    }

    /// All shifts for one layer that intersect `[from, to)`, with overrides applied.
    pub fn layer_shifts(&self, layer: Layer, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<Shift> {
        let (from, to) = clamp_window(from, to);
        let mut shifts = self.rotation_shifts(layer, from, to);
        let active_end = self.end.unwrap_or(DateTime::<Utc>::MAX_UTC);
        for o in self.overrides.iter().filter(|o| o.layer == layer) {
            let start = o.start.max(self.start);
            let end = o.end.min(active_end);
            if end <= start || end <= from || start >= to {
                continue;
            }
            shifts = carve(shifts, start, end);
            shifts.push(Shift { layer, user_id: o.user_id, start, end, source: ShiftSource::Override { override_id: o.id } });
        }
        shifts.retain(|s| s.end > from && s.start < to);
        shifts.sort_by_key(|s| s.start);
        shifts
    }

    /// Shifts for both layers in `[from, to)`, ordered by start time then layer.
    pub fn shifts(&self, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<Shift> {
        let mut all: Vec<Shift> = Layer::ALL.iter().flat_map(|layer| self.layer_shifts(*layer, from, to)).collect();
        all.sort_by(|a, b| a.start.cmp(&b.start).then(a.layer.as_str().cmp(b.layer.as_str())));
        all
    }

    /// Who holds `layer` at instant `t`.
    pub fn on_call_at(&self, layer: Layer, t: DateTime<Utc>) -> Option<Shift> {
        self.layer_shifts(layer, t, t + Duration::seconds(1)).into_iter().find(|s| s.start <= t && t < s.end)
    }

    /// Every instant in `[from, to)` where the person holding `layer` changes, including the
    /// schedule starting (previous = None) and coverage ending (user = None).
    pub fn transitions(&self, layer: Layer, from: DateTime<Utc>, to: DateTime<Utc>) -> Vec<Transition> {
        let probe_from = from - Duration::seconds(1);
        let shifts = self.layer_shifts(layer, probe_from, to);
        let mut out = Vec::new();
        let mut previous: Option<&Shift> = None;
        for shift in &shifts {
            let prev_user = previous.filter(|p| p.end == shift.start).map(|p| p.user_id);
            if let Some(p) = previous {
                if p.end < shift.start && p.end >= from && p.end < to {
                    out.push(Transition { layer, at: p.end, previous_user_id: Some(p.user_id), user_id: None, shift: None });
                }
            }
            if shift.start >= from && shift.start < to && prev_user != Some(shift.user_id) {
                out.push(Transition { layer, at: shift.start, previous_user_id: prev_user, user_id: Some(shift.user_id), shift: Some(shift.clone()) });
            }
            previous = Some(shift);
        }
        if let Some(p) = previous {
            if p.end >= from && p.end < to && !shifts.iter().any(|s| s.start == p.end) {
                out.push(Transition { layer, at: p.end, previous_user_id: Some(p.user_id), user_id: None, shift: None });
            }
        }
        out
    }

    /// Whether `user` holds `layer` for the whole of `[start, end)`, with no gap and nobody else.
    pub fn holds(&self, layer: Layer, user: Uuid, start: DateTime<Utc>, end: DateTime<Utc>) -> bool {
        if end <= start {
            return false;
        }
        let mut covered_until = start;
        for shift in self.layer_shifts(layer, start, end) {
            if shift.start > covered_until || shift.user_id != user {
                return false;
            }
            covered_until = covered_until.max(shift.end);
            if covered_until >= end {
                return true;
            }
        }
        false
    }

    /// The next instant after `t` at which the rotation (not an override) hands over `layer`.
    pub fn next_rotation_boundary(&self, layer: Layer, t: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let horizon = t + Duration::days(MAX_WINDOW_DAYS);
        self.rotation_shifts(layer, t, horizon).into_iter().map(|s| s.end).find(|end| *end > t).filter(|end| self.end.is_none_or(|e| *end < e))
    }
}

pub fn validate_version(version: &RotationVersion) -> Result<(), SpecError> {
    if version.interval == 0 {
        return Err(SpecError::ZeroInterval);
    }
    if version.primary.is_empty() {
        return Err(SpecError::NoPrimaryParticipants);
    }
    for (layer, list) in [("primary", &version.primary), ("secondary", &version.secondary)] {
        let mut seen = std::collections::HashSet::new();
        if !list.iter().all(|id| seen.insert(*id)) {
            return Err(SpecError::DuplicateParticipant(layer));
        }
    }
    Ok(())
}

fn clamp_window(from: DateTime<Utc>, to: DateTime<Utc>) -> (DateTime<Utc>, DateTime<Utc>) {
    let max_to = from + Duration::days(MAX_WINDOW_DAYS);
    (from, to.min(max_to).max(from))
}

/// Remove `[start, end)` from every shift, splitting shifts that straddle it.
fn carve(shifts: Vec<Shift>, start: DateTime<Utc>, end: DateTime<Utc>) -> Vec<Shift> {
    let mut out = Vec::with_capacity(shifts.len() + 1);
    for s in shifts {
        if s.end <= start || s.start >= end {
            out.push(s);
            continue;
        }
        if s.start < start {
            out.push(Shift { end: start, ..s.clone() });
        }
        if s.end > end {
            out.push(Shift { start: end, ..s });
        }
    }
    out
}

/// Convert a local wall-clock time to UTC, resolving DST gaps forward and folds to the earlier
/// instant.
pub fn resolve_local(tz: Tz, local: NaiveDateTime) -> DateTime<Utc> {
    match tz.from_local_datetime(&local) {
        LocalResult::Single(dt) => dt.with_timezone(&Utc),
        LocalResult::Ambiguous(earliest, _) => earliest.with_timezone(&Utc),
        LocalResult::None => {
            // Inside a DST gap: the first valid wall-clock minute after it. Real-world gaps are
            // at most a few hours, so this loop is short and only runs on gap days.
            let mut probe = local;
            for _ in 0..(4 * 60) {
                probe += Duration::minutes(1);
                if let Some(dt) = tz.from_local_datetime(&probe).earliest() {
                    return dt.with_timezone(&Utc);
                }
            }
            Utc.from_utc_datetime(&local)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn user(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn local(tz: Tz, y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        let naive = NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, min, 0).unwrap();
        resolve_local(tz, naive)
    }

    fn version(from: DateTime<Utc>, interval: u32, unit: RotationUnit, primary: &[u128], secondary: &[u128]) -> RotationVersion {
        RotationVersion {
            effective_from: from,
            interval,
            unit,
            primary: primary.iter().map(|n| user(*n)).collect(),
            secondary: secondary.iter().map(|n| user(*n)).collect(),
        }
    }

    const RIKY: u128 = 1;
    const ANDI: u128 = 2;
    const BUDI: u128 = 3;

    fn weekly_jakarta() -> ScheduleSpec {
        let tz: Tz = "Asia/Jakarta".parse().unwrap();
        let start = local(tz, 2026, 11, 3, 9, 0);
        let end = local(tz, 2027, 2, 3, 9, 0);
        ScheduleSpec::new(tz, start, Some(end), vec![version(start, 7, RotationUnit::Days, &[RIKY, ANDI, BUDI], &[])], vec![])
    }

    #[test]
    fn weekly_rotation_follows_participant_order() {
        let spec = weekly_jakarta();
        let tz = spec.timezone;
        let at = |d: (i32, u32, u32), h| spec.on_call_at(Layer::Primary, local(tz, d.0, d.1, d.2, h, 0)).map(|s| s.user_id);
        assert_eq!(at((2026, 11, 3), 9), Some(user(RIKY)));
        assert_eq!(at((2026, 11, 10), 8), Some(user(RIKY)));
        assert_eq!(at((2026, 11, 10), 9), Some(user(ANDI)));
        assert_eq!(at((2026, 11, 17), 9), Some(user(BUDI)));
        assert_eq!(at((2026, 11, 24), 9), Some(user(RIKY)));
    }

    #[test]
    fn boundaries_are_half_open() {
        let spec = weekly_jakarta();
        let boundary = local(spec.timezone, 2026, 11, 10, 9, 0);
        let before = spec.on_call_at(Layer::Primary, boundary - Duration::nanoseconds(1)).unwrap();
        let at = spec.on_call_at(Layer::Primary, boundary).unwrap();
        assert_eq!(before.user_id, user(RIKY));
        assert_eq!(before.end, boundary);
        assert_eq!(at.user_id, user(ANDI));
        assert_eq!(at.start, boundary);
    }

    #[test]
    fn nobody_is_on_call_before_start_or_at_end() {
        let spec = weekly_jakarta();
        assert!(spec.on_call_at(Layer::Primary, spec.start - Duration::seconds(1)).is_none());
        let end = spec.end.unwrap();
        assert!(spec.on_call_at(Layer::Primary, end).is_none());
        let last = spec.on_call_at(Layer::Primary, end - Duration::seconds(1)).unwrap();
        assert_eq!(last.end, end);
    }

    #[test]
    fn final_shift_is_clipped_by_schedule_end() {
        let tz: Tz = "UTC".parse().unwrap();
        let start = local(tz, 2026, 1, 1, 0, 0);
        let end = start + Duration::days(10);
        let spec = ScheduleSpec::new(tz, start, Some(end), vec![version(start, 7, RotationUnit::Days, &[RIKY, ANDI], &[])], vec![]);
        let shifts = spec.layer_shifts(Layer::Primary, start, end + Duration::days(30));
        assert_eq!(shifts.len(), 2);
        assert_eq!(shifts[1].start, start + Duration::days(7));
        assert_eq!(shifts[1].end, end);
    }

    #[test]
    fn open_ended_schedules_keep_rotating() {
        let tz: Tz = "UTC".parse().unwrap();
        let start = local(tz, 2026, 1, 1, 9, 0);
        let spec = ScheduleSpec::new(tz, start, None, vec![version(start, 1, RotationUnit::Days, &[RIKY, ANDI, BUDI], &[])], vec![]);
        // 10 years later: 3652 days -> index 3652 % 3 == 1 -> Andi.
        let t = start + Duration::days(3652) + Duration::hours(1);
        assert_eq!(spec.on_call_at(Layer::Primary, t).unwrap().user_id, user(ANDI));
        assert!(spec.next_rotation_boundary(Layer::Primary, t).is_some());
    }

    #[test]
    fn daily_rotation_keeps_local_time_across_dst() {
        // America/New_York springs forward on 8 March 2026 and falls back on 1 November 2026.
        let tz: Tz = "America/New_York".parse().unwrap();
        let start = local(tz, 2026, 3, 6, 9, 0);
        let spec = ScheduleSpec::new(tz, start, None, vec![version(start, 1, RotationUnit::Days, &[RIKY, ANDI, BUDI], &[])], vec![]);
        let shifts = spec.layer_shifts(Layer::Primary, start, start + Duration::days(5));
        for s in &shifts {
            assert_eq!(s.start.with_timezone(&tz).format("%H:%M").to_string(), "09:00");
        }
        // The shift spanning the spring-forward night is 23 hours long.
        let short = shifts.iter().find(|s| s.start.with_timezone(&tz).date_naive() == NaiveDate::from_ymd_opt(2026, 3, 7).unwrap()).unwrap();
        assert_eq!(short.end - short.start, Duration::hours(23));

        let autumn = local(tz, 2026, 10, 31, 9, 0);
        let long = spec.on_call_at(Layer::Primary, autumn + Duration::hours(1)).unwrap();
        assert_eq!(long.end - long.start, Duration::hours(25));
    }

    #[test]
    fn hourly_rotation_uses_absolute_time_across_dst() {
        let tz: Tz = "America/New_York".parse().unwrap();
        let start = local(tz, 2026, 3, 8, 0, 0);
        let spec = ScheduleSpec::new(tz, start, None, vec![version(start, 12, RotationUnit::Hours, &[RIKY, ANDI], &[])], vec![]);
        let shifts = spec.layer_shifts(Layer::Primary, start, start + Duration::days(2));
        assert!(shifts.iter().all(|s| s.end - s.start == Duration::hours(12)));
    }

    #[test]
    fn handover_inside_a_dst_gap_moves_to_the_first_valid_instant() {
        let tz: Tz = "America/New_York".parse().unwrap();
        // 02:30 does not exist on 8 March 2026 in New York.
        let gap = NaiveDate::from_ymd_opt(2026, 3, 8).unwrap().and_hms_opt(2, 30, 0).unwrap();
        let resolved = resolve_local(tz, gap);
        assert_eq!(resolved.with_timezone(&tz).format("%H:%M").to_string(), "03:00");

        let start = local(tz, 2026, 3, 6, 2, 30);
        let spec = ScheduleSpec::new(tz, start, None, vec![version(start, 1, RotationUnit::Days, &[RIKY, ANDI], &[])], vec![]);
        let shifts = spec.layer_shifts(Layer::Primary, start, start + Duration::days(3) + Duration::hours(12));
        let starts: Vec<String> = shifts.iter().map(|s| s.start.with_timezone(&tz).format("%d %H:%M").to_string()).collect();
        assert_eq!(starts, vec!["06 02:30", "07 02:30", "08 03:00", "09 02:30"]);
    }

    #[test]
    fn primary_and_secondary_rotate_independently() {
        let tz: Tz = "UTC".parse().unwrap();
        let start = local(tz, 2026, 1, 1, 0, 0);
        let spec = ScheduleSpec::new(tz, start, None, vec![version(start, 1, RotationUnit::Weeks, &[RIKY, ANDI, BUDI], &[BUDI, RIKY])], vec![]);
        let week = |n: i64| start + Duration::weeks(n) + Duration::hours(1);
        let pair = |n| (spec.on_call_at(Layer::Primary, week(n)).unwrap().user_id, spec.on_call_at(Layer::Secondary, week(n)).unwrap().user_id);
        assert_eq!(pair(0), (user(RIKY), user(BUDI)));
        assert_eq!(pair(1), (user(ANDI), user(RIKY)));
        assert_eq!(pair(2), (user(BUDI), user(BUDI)));
        assert_eq!(pair(3), (user(RIKY), user(RIKY)));
    }

    #[test]
    fn empty_secondary_means_no_secondary() {
        let spec = weekly_jakarta();
        assert!(spec.on_call_at(Layer::Secondary, spec.start + Duration::days(1)).is_none());
    }

    #[test]
    fn override_replaces_assignee_only_for_its_window() {
        let mut spec = weekly_jakarta();
        let tz = spec.timezone;
        let o_start = local(tz, 2026, 11, 5, 0, 0);
        let o_end = local(tz, 2026, 11, 6, 0, 0);
        spec.overrides.push(Override { id: user(99), layer: Layer::Primary, user_id: user(BUDI), start: o_start, end: o_end });

        let shifts = spec.layer_shifts(Layer::Primary, spec.start, local(tz, 2026, 11, 10, 9, 0));
        let summary: Vec<(Uuid, DateTime<Utc>, DateTime<Utc>)> = shifts.iter().map(|s| (s.user_id, s.start, s.end)).collect();
        assert_eq!(summary, vec![(user(RIKY), spec.start, o_start), (user(BUDI), o_start, o_end), (user(RIKY), o_end, local(tz, 2026, 11, 10, 9, 0)),]);
        assert!(matches!(shifts[1].source, ShiftSource::Override { .. }));
    }

    #[test]
    fn later_override_wins_and_overrides_stay_inside_the_schedule() {
        let tz: Tz = "UTC".parse().unwrap();
        let start = local(tz, 2026, 1, 1, 0, 0);
        let end = start + Duration::days(7);
        let spec = ScheduleSpec::new(
            tz,
            start,
            Some(end),
            vec![version(start, 7, RotationUnit::Days, &[RIKY], &[])],
            vec![
                Override { id: user(90), layer: Layer::Primary, user_id: user(ANDI), start: start - Duration::days(1), end: start + Duration::days(2) },
                Override { id: user(91), layer: Layer::Primary, user_id: user(BUDI), start: start + Duration::days(1), end: end + Duration::days(5) },
            ],
        );
        let shifts = spec.layer_shifts(Layer::Primary, start - Duration::days(3), end + Duration::days(10));
        let summary: Vec<(Uuid, i64, i64)> = shifts.iter().map(|s| (s.user_id, (s.start - start).num_days(), (s.end - start).num_days())).collect();
        assert_eq!(summary, vec![(user(ANDI), 0, 1), (user(BUDI), 1, 7)]);
    }

    #[test]
    fn editing_creates_a_new_version_without_rewriting_history() {
        let mut spec = weekly_jakarta();
        let tz = spec.timezone;
        let history_point = local(tz, 2026, 11, 12, 12, 0);
        let before = spec.on_call_at(Layer::Primary, history_point).unwrap();

        // On 20 November, Andi leaves the rotation. Edit applies from that moment.
        let edit_at = local(tz, 2026, 11, 20, 15, 0);
        spec.versions.push(version(edit_at, 7, RotationUnit::Days, &[RIKY, BUDI], &[]));
        let spec = ScheduleSpec::new(spec.timezone, spec.start, spec.end, spec.versions, spec.overrides);

        assert_eq!(spec.on_call_at(Layer::Primary, history_point).unwrap(), before);
        // Week index 2 (17 Nov) under the new list [Riky, Budi] -> index 2 % 2 = 0 -> Riky.
        let after = spec.on_call_at(Layer::Primary, edit_at).unwrap();
        assert_eq!(after.user_id, user(RIKY));
        assert_eq!(after.start, edit_at);
        assert_eq!(after.end, local(tz, 2026, 11, 24, 9, 0));
        // Budi's shift before the edit is preserved and ends exactly at the edit.
        let budi = spec.on_call_at(Layer::Primary, edit_at - Duration::seconds(1)).unwrap();
        assert_eq!((budi.user_id, budi.end), (user(BUDI), edit_at));
    }

    #[test]
    fn transitions_report_only_real_changes() {
        let tz: Tz = "UTC".parse().unwrap();
        let start = local(tz, 2026, 1, 1, 0, 0);
        let end = start + Duration::days(4);
        let spec = ScheduleSpec::new(tz, start, Some(end), vec![version(start, 1, RotationUnit::Days, &[RIKY, ANDI], &[])], vec![]);
        let ts = spec.transitions(Layer::Primary, start - Duration::days(1), end + Duration::days(1));
        let summary: Vec<(i64, Option<Uuid>, Option<Uuid>)> = ts.iter().map(|t| ((t.at - start).num_days(), t.previous_user_id, t.user_id)).collect();
        assert_eq!(
            summary,
            vec![
                (0, None, Some(user(RIKY))),
                (1, Some(user(RIKY)), Some(user(ANDI))),
                (2, Some(user(ANDI)), Some(user(RIKY))),
                (3, Some(user(RIKY)), Some(user(ANDI))),
                (4, Some(user(ANDI)), None),
            ]
        );

        let single = ScheduleSpec::new(tz, start, None, vec![version(start, 1, RotationUnit::Days, &[RIKY], &[])], vec![]);
        assert_eq!(single.transitions(Layer::Primary, start + Duration::hours(1), start + Duration::days(10)).len(), 0);
    }

    #[test]
    fn transition_window_is_half_open() {
        let spec = weekly_jakarta();
        let boundary = local(spec.timezone, 2026, 11, 10, 9, 0);
        assert_eq!(spec.transitions(Layer::Primary, boundary, boundary + Duration::seconds(1)).len(), 1);
        assert_eq!(spec.transitions(Layer::Primary, boundary - Duration::hours(1), boundary).len(), 0);
    }

    #[test]
    fn next_rotation_boundary_respects_end() {
        let spec = weekly_jakarta();
        let tz = spec.timezone;
        let t = local(tz, 2026, 11, 4, 0, 0);
        assert_eq!(spec.next_rotation_boundary(Layer::Primary, t), Some(local(tz, 2026, 11, 10, 9, 0)));
        let near_end = spec.end.unwrap() - Duration::days(1);
        assert_eq!(spec.next_rotation_boundary(Layer::Primary, near_end), None);
    }

    #[test]
    fn results_are_deterministic_and_independent_of_query_window() {
        let spec = weekly_jakarta();
        let t = local(spec.timezone, 2026, 12, 25, 3, 0);
        let direct = spec.on_call_at(Layer::Primary, t).unwrap();
        let from_wide = spec.layer_shifts(Layer::Primary, spec.start, spec.end.unwrap()).into_iter().find(|s| s.start <= t && t < s.end).unwrap();
        assert_eq!(direct, from_wide);
    }

    #[test]
    fn validation_rejects_bad_configuration() {
        let tz: Tz = "UTC".parse().unwrap();
        let start = local(tz, 2026, 1, 1, 0, 0);
        let bad_end = ScheduleSpec::new(tz, start, Some(start), vec![version(start, 1, RotationUnit::Days, &[RIKY], &[])], vec![]);
        assert_eq!(bad_end.validate(), Err(SpecError::EndBeforeStart));
        let empty = ScheduleSpec::new(tz, start, None, vec![version(start, 1, RotationUnit::Days, &[], &[])], vec![]);
        assert_eq!(empty.validate(), Err(SpecError::NoPrimaryParticipants));
        let zero = ScheduleSpec::new(tz, start, None, vec![version(start, 0, RotationUnit::Days, &[RIKY], &[])], vec![]);
        assert_eq!(zero.validate(), Err(SpecError::ZeroInterval));
        let dup = ScheduleSpec::new(tz, start, None, vec![version(start, 1, RotationUnit::Days, &[RIKY, RIKY], &[])], vec![]);
        assert_eq!(dup.validate(), Err(SpecError::DuplicateParticipant("primary")));
    }

    #[test]
    fn large_windows_are_clamped() {
        let spec = weekly_jakarta();
        let shifts = spec.layer_shifts(Layer::Primary, spec.start, spec.start + Duration::days(100_000));
        assert!(shifts.last().unwrap().end <= spec.start + Duration::days(MAX_WINDOW_DAYS) + Duration::days(7));
    }

    #[test]
    fn holds_requires_the_whole_window() {
        let spec = weekly_jakarta();
        let tz = spec.timezone;
        let (riky_from, riky_to) = (local(tz, 2026, 11, 3, 9, 0), local(tz, 2026, 11, 10, 9, 0));
        assert!(spec.holds(Layer::Primary, user(RIKY), riky_from, riky_to));
        assert!(spec.holds(Layer::Primary, user(RIKY), riky_from + Duration::days(1), riky_to - Duration::days(1)));
        // Runs into Andi's week.
        assert!(!spec.holds(Layer::Primary, user(RIKY), riky_from, riky_to + Duration::hours(1)));
        // Starts before the schedule exists.
        assert!(!spec.holds(Layer::Primary, user(RIKY), riky_from - Duration::hours(1), riky_to));
        assert!(!spec.holds(Layer::Primary, user(ANDI), riky_from, riky_to));
        assert!(!spec.holds(Layer::Primary, user(RIKY), riky_to, riky_from));
        // An override hands part of the week to Budi.
        let mut with_override = spec.clone();
        with_override.overrides.push(Override {
            id: Uuid::nil(),
            layer: Layer::Primary,
            user_id: user(BUDI),
            start: riky_from + Duration::days(2),
            end: riky_from + Duration::days(3),
        });
        assert!(!with_override.holds(Layer::Primary, user(RIKY), riky_from, riky_to));
        assert!(with_override.holds(Layer::Primary, user(BUDI), riky_from + Duration::days(2), riky_from + Duration::days(3)));
    }

    #[test]
    fn first_version_covers_an_earlier_start() {
        // The start was moved two days earlier before the schedule began; the stored version
        // still carries the old start as its effective_from.
        let tz: Tz = "UTC".parse().unwrap();
        let old_start = local(tz, 2026, 1, 10, 9, 0);
        let new_start = old_start - Duration::days(2);
        let spec = ScheduleSpec::new(tz, new_start, None, vec![version(old_start, 1, RotationUnit::Days, &[RIKY, ANDI], &[])], vec![]);
        assert_eq!(spec.on_call_at(Layer::Primary, new_start).map(|s| s.user_id), Some(user(RIKY)));
        assert_eq!(spec.on_call_at(Layer::Primary, new_start + Duration::days(1)).map(|s| s.user_id), Some(user(ANDI)));
    }
}
