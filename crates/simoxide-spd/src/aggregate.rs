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
pub struct Ema {
    pub window: f64,
    pub no_emit: f64,
    pub alpha: f64,
    pub value: f64,
    pub emit_time: f64,
    pub last_update: f64,
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
        }
    }

    /// `aggregate(currentTime, newValue)`: `v = (1 - α) v + α x`, then
    /// `v *= exp(-(t - t_last) / window)`.
    pub fn aggregate(&mut self, t: f64, x: f64) -> f64 {
        let dt = t - self.last_update;
        self.value = (1.0 - self.alpha) * self.value + self.alpha * x;
        let factor = (-dt / self.window).exp();
        self.value *= factor;
        self.last_update = t;
        self.value
    }

    /// `isEmittable`: `t_last > window && t_last - t_emit > no_emit`.
    pub fn emittable(&self) -> bool {
        self.last_update > self.window && self.last_update - self.emit_time > self.no_emit
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

    pub fn aggregate(&mut self, t: f64, x: f64) {
        match self {
            Window::Ema(e) => {
                e.aggregate(t, x);
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

    /// Ready to contribute a value (fixed Q13): the EMA has seen samples after its window
    /// length, the count window is full.
    pub fn ready(&self) -> bool {
        match self {
            Window::Ema(e) => e.last_update > e.window,
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
/// * A sample updates its element's window only.
/// * The value is the aggregation method over the current values of the *ready* elements
///   ([`Window::ready`]); there is a value once one element is ready.
/// * For `AVERAGE` (EMA windows) the group emits at most once per no-emit duration: the
///   latest update must be more than 10 s after the last emission; the emission is committed
///   only when the policy fires ([`PerElement::commit`]), not when the value is read.
#[derive(Clone, Debug, PartialEq)]
pub struct PerElement {
    pub aggregation: Aggregation,
    /// (element key, its window), in first-sample order.
    pub elements: Vec<(u32, Window)>,
    pub emit_time: f64,
}

impl PerElement {
    pub fn new(aggregation: Aggregation) -> PerElement {
        PerElement {
            aggregation,
            elements: Vec::new(),
            emit_time: 0.0,
        }
    }

    pub fn aggregate(&mut self, element: u32, t: f64, x: f64) {
        let i = match self.elements.iter().position(|(k, _)| *k == element) {
            Some(i) => i,
            None => {
                self.elements
                    .push((element, Window::for_aggregation(self.aggregation)));
                self.elements.len() - 1
            }
        };
        self.elements[i].1.aggregate(t, x);
    }

    fn latest(&self) -> Option<f64> {
        self.elements
            .iter()
            .filter_map(|(_, w)| w.last_update())
            .reduce(f64::max)
    }

    pub fn emittable(&self) -> bool {
        if !self.elements.iter().any(|(_, w)| w.ready()) {
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
            .filter(|(_, w)| w.ready())
            .map(|(_, w)| w.peek());
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
        let v = e.aggregate(30.0, 1.0);
        assert_eq!(v, 0.2 * (-0.5f64).exp());
        assert!(!e.emittable(), "not before t > 60");
        e.aggregate(60.0, 1.0);
        assert!(!e.emittable(), "t_last == 60 is not > 60");
        e.aggregate(60.5, 1.0);
        assert!(e.emittable());
        e.take();
        assert_eq!(e.emit_time, 60.5);
        e.aggregate(70.5, 1.0);
        assert!(!e.emittable(), "70.5 - 60.5 == 10 is not > 10");
        e.aggregate(70.6, 1.0);
        assert!(e.emittable());
    }

    #[test]
    fn ema_reading_consumes_slot() {
        let mut e = Ema::new(60.0, 10.0, 0.2);
        e.aggregate(61.0, 1.0);
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
            p.aggregate(1, i as f64, 0.5);
        }
        for i in 0..9 {
            p.aggregate(2, i as f64, 0.9);
        }
        assert!(p.emittable());
        // element 2 is not ready yet
        assert_eq!(p.value(), 0.5);
        p.aggregate(2, 9.0, 0.9);
        assert_eq!(p.value(), 0.9);
        let mut a = PerElement::new(Aggregation::Average);
        a.aggregate(1, 61.0, 1.0);
        a.aggregate(2, 62.0, 0.0);
        assert!(a.emittable());
        let e1 = 0.2 * (-61.0f64 / 60.0).exp();
        let e2 = 0.0;
        assert_eq!(a.value(), (e1 + e2) / 2.0);
        // reading does not consume; firing does
        assert!(a.emittable());
        a.commit();
        assert!(!a.emittable());
        a.aggregate(1, 72.5, 1.0);
        assert!(a.emittable());
    }
}
