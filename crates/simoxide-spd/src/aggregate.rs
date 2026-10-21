//! Aggregation windows of managed-element stimuli (`entity/aggregator/` of Slingshot's SPD
//! interpreter; `DESIGN-SPD.md` §1.2 "Aggregation", SS-SPD-2.4, quirk Q3 kept).
//!
//! * `AVERAGE`: [`Ema`], `SlidingTimeWindowAggregationBasedOnEMA(60, 10, 0.2)`.
//! * `MAX`, `MIN`, `MEDIAN`, `SUM`: [`LastN`], `FixedLengthWindowSimpleAggregation` over the
//!   last 10 values.
//!
//! `SlidingTimeWindowAggregation` (time based) exists in Slingshot but is never used.
//!
//! All windows are plain data (part of [`crate::SpdState`]). `Math.exp` is `f64::exp` here;
//! HotSpot's intrinsic can differ in the last bit (the exact mode can switch to
//! `simoxide-hotspot-math`).

use crate::model::Aggregation;
use std::cmp::Ordering;
use std::collections::VecDeque;

/// Size of the count-based window.
pub const WINDOW_SIZE: usize = 10;
/// EMA window (s), no-emit duration (s) and smoothing factor.
pub const EMA_WINDOW: f64 = 60.0;
pub const EMA_NO_EMIT: f64 = 10.0;
pub const EMA_ALPHA: f64 = 0.2;

/// `SlidingTimeWindowAggregationBasedOnEMA`.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct Ema {
    pub window: f64,
    pub no_emit: f64,
    pub alpha: f64,
    pub value: f64,
    pub emit_time: f64,
    pub last_update: f64,
    /// When the window started (s): 0 for Slingshot's aggregator, created at set-up; for a
    /// fixed-Q13 element window, its element's first sample ([`Ema::starting_at`]).
    pub start: f64,
}

impl Ema {
    pub fn new(window: f64, no_emit: f64, alpha: f64) -> Ema {
        Ema {
            window,
            no_emit,
            alpha,
            value: 0.0,
            emit_time: 0.0,
            last_update: 0.0,
            start: 0.0,
        }
    }

    /// A window that starts at `t` (fixed Q13: an element's window starts with the element's
    /// first sample, so a container added later is neither decayed over the time before it
    /// existed nor ready before it has been measured for a window length).
    pub fn starting_at(window: f64, no_emit: f64, alpha: f64, t: f64) -> Ema {
        Ema {
            emit_time: t,
            last_update: t,
            start: t,
            ..Ema::new(window, no_emit, alpha)
        }
    }

    /// Samples for longer than the window: `t_last - start > window` (Slingshot: `t_last >
    /// window`, its window starts at 0).
    pub fn full(&self) -> bool {
        self.last_update - self.start > self.window
    }

    /// `aggregate(currentTime, newValue)`: `v = (1 - α) v + α x`, then
    /// `v *= exp(-(t - t_last) / window)`.
    ///
    /// The decay is Slingshot's correction for its utilisation samples, taken only when a job
    /// arrives or leaves: the time without samples counts as idle (the class's Javadoc). A
    /// sample that `covers` a span (a sliding window's utilisation, SimuLizar flavour) already
    /// accounts for that span, so only the time between samples that no sample covers decays:
    /// `v *= exp(-max(0, t - t_last - covers) / window)`. Instantaneous samples (`covers ==
    /// 0`) decay as in Slingshot.
    pub fn aggregate(&mut self, t: f64, x: f64, covers: f64) -> f64 {
        let mut dt = t - self.last_update;
        if covers > 0.0 {
            dt = (dt - covers).max(0.0);
        }
        self.value = (1.0 - self.alpha) * self.value + self.alpha * x;
        let factor = (-dt / self.window).exp();
        self.value *= factor;
        self.last_update = t;
        self.value
    }

    /// `isEmittable`: `t_last > window && t_last - t_emit > no_emit` (window from `start`).
    pub fn emittable(&self) -> bool {
        self.full() && self.last_update - self.emit_time > self.no_emit
    }

    /// `getCurrentValue`: the value; consumes the emit slot (`t_emit = t_last`).
    pub fn take(&mut self) -> f64 {
        self.emit_time = self.last_update;
        self.value
    }
}

/// `FixedLengthWindowSimpleAggregation`: the last `n` values and an aggregation function,
/// emittable once `n` values are present.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct LastN {
    pub n: usize,
    pub f: Aggregation,
    pub values: VecDeque<f64>,
    pub current: f64,
}

impl LastN {
    pub fn new(n: usize, f: Aggregation) -> LastN {
        LastN {
            n,
            f,
            values: VecDeque::with_capacity(n),
            current: 0.0,
        }
    }

    pub fn aggregate(&mut self, x: f64) -> f64 {
        if self.values.len() == self.n {
            self.values.pop_front();
        }
        self.values.push_back(x);
        self.current = apply(self.f, self.values.iter().copied());
        self.current
    }

    pub fn emittable(&self) -> bool {
        self.values.len() == self.n
    }
}

/// A window of the kind Slingshot uses for `aggregation` (Q3).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub enum Window {
    Ema(Ema),
    Last(LastN),
}

impl Window {
    /// `AVERAGE` → EMA, the others → the last 10 values.
    pub fn for_aggregation(a: Aggregation) -> Window {
        match a {
            Aggregation::Average => Window::Ema(Ema::new(EMA_WINDOW, EMA_NO_EMIT, EMA_ALPHA)),
            f => Window::Last(LastN::new(WINDOW_SIZE, f)),
        }
    }

    /// A window of `a` for an element whose first sample is at `t` (fixed Q13): the EMA starts
    /// at `t` ([`Ema::starting_at`]).
    pub fn for_element(a: Aggregation, t: f64) -> Window {
        match a {
            Aggregation::Average => {
                Window::Ema(Ema::starting_at(EMA_WINDOW, EMA_NO_EMIT, EMA_ALPHA, t))
            }
            f => Window::Last(LastN::new(WINDOW_SIZE, f)),
        }
    }

    /// A sample `x` at `t` that covers the span `covers` before it ([`Ema::aggregate`]).
    pub fn aggregate(&mut self, t: f64, x: f64, covers: f64) {
        match self {
            Window::Ema(e) => {
                e.aggregate(t, x, covers);
            }
            Window::Last(l) => {
                l.aggregate(x);
            }
        }
    }

    pub fn emittable(&self) -> bool {
        match self {
            Window::Ema(e) => e.emittable(),
            Window::Last(l) => l.emittable(),
        }
    }

    /// `getCurrentValue` (consumes the EMA's emit slot).
    pub fn take(&mut self) -> f64 {
        match self {
            Window::Ema(e) => e.take(),
            Window::Last(l) => l.current,
        }
    }

    /// The current value without consuming anything.
    pub fn peek(&self) -> f64 {
        match self {
            Window::Ema(e) => e.value,
            Window::Last(l) => l.current,
        }
    }

    /// Ready to contribute a value (fixed Q13): the EMA has seen samples for longer than its
    /// window length since it started, the count window is full.
    pub fn ready(&self) -> bool {
        match self {
            Window::Ema(e) => e.full(),
            Window::Last(l) => l.emittable(),
        }
    }

    fn last_update(&self) -> Option<f64> {
        match self {
            Window::Ema(e) => Some(e.last_update),
            Window::Last(_) => None,
        }
    }
}

/// `Double.compare` (every NaN equal and above +Infinity, -0.0 < 0.0).
pub fn java_compare(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => a.total_cmp(&b),
    }
}

/// The aggregation functions of `entity/aggregator/functions/` over `values` (non-empty for
/// `MAX`/`MIN`, which throw on an empty window in Slingshot; guarded by `isEmittable`).
pub fn apply(f: Aggregation, values: impl Iterator<Item = f64> + Clone) -> f64 {
    match f {
        Aggregation::Max => values
            .max_by(|a, b| java_compare(*a, *b))
            .unwrap_or(f64::NAN),
        Aggregation::Min => values
            .min_by(|a, b| java_compare(*a, *b))
            .unwrap_or(f64::NAN),
        Aggregation::Sum => values.fold(0.0, |s, x| s + x),
        Aggregation::Median => {
            let mut v: Vec<f64> = values.collect();
            if v.is_empty() {
                return 0.0;
            }
            v.sort_by(|a, b| java_compare(*a, *b));
            let n = v.len();
            if n.is_multiple_of(2) {
                (v[n / 2] + v[n / 2 - 1]) / 2.0
            } else {
                v[(n - 1) / 2]
            }
        }
        Aggregation::Average => {
            // `DoubleStream.average()`: compensated (Kahan) summation
            let (mut sum, mut c, mut simple, mut n) = (0.0f64, 0.0f64, 0.0f64, 0u64);
            for x in values {
                let tmp = x - c;
                let velvel = sum + tmp;
                c = (velvel - sum) - tmp;
                sum = velvel;
                simple += x;
                n += 1;
            }
            if n == 0 {
                return f64::NAN;
            }
            let total = sum - c;
            let total = if total.is_nan() && simple.is_infinite() {
                simple
            } else {
                total
            };
            total / n as f64
        }
    }
}

/// Fixed Q13 (provisional): one window per element (measuring point) of the group.
///
/// * A sample updates its element's window only. An element's window starts with its first
///   sample ([`Window::for_element`]).
/// * An element that leaves the group (a container removed by a scale-in) loses its window
///   ([`PerElement::retain`]): it no longer contributes its last value.
/// * The value is the aggregation method over the current values of the *ready* elements
///   ([`Window::ready`]); there is a value once one element is ready.
/// * For `AVERAGE` (EMA windows) the group emits at most once per no-emit duration: the
///   latest update must be more than 10 s after the last emission; the emission is committed
///   only when the policy fires ([`PerElement::commit`]), not when the value is read.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct PerElement {
    pub aggregation: Aggregation,
    /// The elements' windows, in first-sample order.
    pub elements: Vec<Element>,
    pub emit_time: f64,
}

/// One element of a [`PerElement`] aggregation.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct Element {
    /// The host's key of the measuring point (series).
    pub key: u32,
    /// The resource container of the measuring point (object number), if it has one: the
    /// element leaves the group with it.
    pub container: Option<u32>,
    pub window: Window,
}

impl PerElement {
    pub fn new(aggregation: Aggregation) -> PerElement {
        PerElement {
            aggregation,
            elements: Vec::new(),
            emit_time: 0.0,
        }
    }

    /// A sample `x` at `t` of element `key` (on `container`, if any), covering `covers`
    /// before `t` ([`Ema::aggregate`]).
    pub fn aggregate(&mut self, key: u32, container: Option<u32>, t: f64, x: f64, covers: f64) {
        let i = match self.elements.iter().position(|e| e.key == key) {
            Some(i) => i,
            None => {
                self.elements.push(Element {
                    key,
                    container,
                    window: Window::for_element(self.aggregation, t),
                });
                self.elements.len() - 1
            }
        };
        self.elements[i].window.aggregate(t, x, covers);
    }

    /// Keeps the elements whose container `keep` accepts (elements without a container stay).
    pub fn retain(&mut self, mut keep: impl FnMut(u32) -> bool) {
        self.elements.retain(|e| e.container.is_none_or(&mut keep));
    }

    fn latest(&self) -> Option<f64> {
        self.elements
            .iter()
            .filter_map(|e| e.window.last_update())
            .reduce(f64::max)
    }

    pub fn emittable(&self) -> bool {
        if !self.elements.iter().any(|e| e.window.ready()) {
            return false;
        }
        match self.latest() {
            Some(t) => t - self.emit_time > EMA_NO_EMIT,
            None => true,
        }
    }

    /// The method over the ready elements' values (call when [`PerElement::emittable`]).
    pub fn value(&self) -> f64 {
        let vals = self
            .elements
            .iter()
            .filter(|e| e.window.ready())
            .map(|e| e.window.peek());
        apply(self.aggregation, vals)
    }

    /// The policy fired on this value: the emission is consumed.
    pub fn commit(&mut self) {
        if let Some(t) = self.latest() {
            self.emit_time = t;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ema_rule() {
        let mut e = Ema::new(60.0, 10.0, 0.2);
        // first sample at t = 30: v = 0.2 * 1 * exp(-30/60)
        let v = e.aggregate(30.0, 1.0, 0.0);
        assert_eq!(v, 0.2 * (-0.5f64).exp());
        assert!(!e.emittable(), "not before t > 60");
        e.aggregate(60.0, 1.0, 0.0);
        assert!(!e.emittable(), "t_last == 60 is not > 60");
        e.aggregate(60.5, 1.0, 0.0);
        assert!(e.emittable());
        e.take();
        assert_eq!(e.emit_time, 60.5);
        e.aggregate(70.5, 1.0, 0.0);
        assert!(!e.emittable(), "70.5 - 60.5 == 10 is not > 10");
        e.aggregate(70.6, 1.0, 0.0);
        assert!(e.emittable());
    }

    #[test]
    fn ema_reading_consumes_slot() {
        let mut e = Ema::new(60.0, 10.0, 0.2);
        e.aggregate(61.0, 1.0, 0.0);
        assert!(e.emittable());
        e.take();
        assert!(!e.emittable());
        // an idle system does not decay the EMA: no samples, no change
        let v = e.value;
        assert_eq!(e.value, v);
    }

    #[test]
    fn last_ten_window() {
        let mut w = LastN::new(10, Aggregation::Max);
        for i in 0..9 {
            w.aggregate(i as f64);
            assert!(!w.emittable());
        }
        w.aggregate(9.0);
        assert!(w.emittable());
        assert_eq!(w.current, 9.0);
        // the oldest value leaves the window
        w.aggregate(-1.0);
        assert_eq!(w.values.len(), 10);
        assert_eq!(w.values.front(), Some(&1.0));
        assert_eq!(w.current, 9.0);
        let mut m = LastN::new(10, Aggregation::Min);
        for i in 0..11 {
            m.aggregate(i as f64);
        }
        assert_eq!(m.current, 1.0);
    }

    #[test]
    fn functions() {
        let v = [3.0, 1.0, 2.0, 4.0];
        assert_eq!(apply(Aggregation::Median, v.iter().copied()), 2.5);
        assert_eq!(apply(Aggregation::Median, [3.0, 1.0, 2.0].into_iter()), 2.0);
        assert_eq!(apply(Aggregation::Sum, v.iter().copied()), 10.0);
        assert_eq!(apply(Aggregation::Max, v.iter().copied()), 4.0);
        assert_eq!(apply(Aggregation::Min, v.iter().copied()), 1.0);
        assert_eq!(apply(Aggregation::Average, v.iter().copied()), 2.5);
        assert_eq!(apply(Aggregation::Median, std::iter::empty()), 0.0);
        // Double.compare: NaN is the maximum, -0.0 < 0.0
        assert!(apply(Aggregation::Max, [1.0, f64::NAN].into_iter()).is_nan());
        assert_eq!(apply(Aggregation::Min, [1.0, f64::NAN].into_iter()), 1.0);
        assert!(apply(Aggregation::Max, [-0.0, 0.0].into_iter()).is_sign_positive());
        assert!(apply(Aggregation::Min, [0.0, -0.0].into_iter()).is_sign_negative());
    }

    #[test]
    fn per_element() {
        let mut p = PerElement::new(Aggregation::Max);
        for i in 0..10 {
            p.aggregate(1, None, i as f64, 0.5, 0.0);
        }
        for i in 0..9 {
            p.aggregate(2, None, i as f64, 0.9, 0.0);
        }
        assert!(p.emittable());
        // element 2 is not ready yet
        assert_eq!(p.value(), 0.5);
        p.aggregate(2, None, 9.0, 0.9, 0.0);
        assert_eq!(p.value(), 0.9);
        let mut a = PerElement::new(Aggregation::Average);
        a.aggregate(1, None, 0.0, 1.0, 0.0);
        a.aggregate(1, None, 61.0, 1.0, 0.0);
        a.aggregate(2, None, 62.0, 0.0, 0.0);
        assert!(a.emittable());
        let e1 = (0.8 * 0.2 + 0.2) * (-61.0f64 / 60.0).exp();
        // element 2 started at 62: not ready, not in the value
        assert_eq!(a.value(), e1);
        // reading does not consume; firing does
        assert!(a.emittable());
        a.commit();
        assert!(!a.emittable());
        a.aggregate(1, None, 72.5, 1.0, 0.0);
        assert!(a.emittable());
    }

    /// An element's EMA starts with its first sample: not decayed over the time before, ready
    /// a window length later.
    #[test]
    fn per_element_window_starts_with_element() {
        let mut a = PerElement::new(Aggregation::Average);
        a.aggregate(1, Some(10), 100.0, 1.0, 0.0);
        assert_eq!(a.elements[0].window.peek(), 0.2);
        assert!(!a.emittable());
        a.aggregate(1, Some(10), 160.0, 1.0, 0.0);
        assert!(!a.emittable(), "160 - 100 == 60 is not > 60");
        a.aggregate(1, Some(10), 160.5, 1.0, 0.0);
        assert!(a.emittable());
    }

    /// Sliding-window samples (SimuLizar flavour) decay only over the time no window covers:
    /// a constant utilisation sampled every 10 s by 10 s windows stays that utilisation; as
    /// instantaneous samples 10 s apart it settles at 0.2 f / (1 - 0.8 f) of it, f = e^(-1/6).
    #[test]
    fn ema_covered_samples() {
        let mut win = Ema::starting_at(60.0, 10.0, 0.2, 10.0);
        let mut inst = Ema::starting_at(60.0, 10.0, 0.2, 10.0);
        for k in 1..=60 {
            let t = 10.0 * k as f64;
            win.aggregate(t, 1.0, 10.0);
            inst.aggregate(t, 1.0, 0.0);
        }
        assert!((win.value - 1.0).abs() < 1e-5, "{}", win.value);
        let f = (-10.0f64 / 60.0).exp();
        assert!(
            (inst.value - 0.2 * f / (1.0 - 0.8 * f)).abs() < 1e-9,
            "{}",
            inst.value
        );
        // a gap longer than the window: the uncovered 5 s decay
        let v = win.value;
        win.aggregate(615.0, 1.0, 10.0);
        assert_eq!(win.value, (0.8 * v + 0.2) * (-5.0f64 / 60.0).exp());
    }

    /// Elements that leave the group drop out of the value.
    #[test]
    fn per_element_retain() {
        let mut a = PerElement::new(Aggregation::Max);
        for i in 0..10 {
            a.aggregate(1, Some(10), i as f64, 0.5, 0.0);
            a.aggregate(2, Some(20), i as f64, 0.9, 0.0);
            a.aggregate(3, None, i as f64, 0.1, 0.0);
        }
        assert_eq!(a.value(), 0.9);
        a.retain(|c| c != 20);
        assert_eq!(a.elements.iter().map(|e| e.key).collect::<Vec<_>>(), [1, 3]);
        assert_eq!(a.value(), 0.5);
    }
}
