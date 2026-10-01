//! Sliding-window utilisation (spec MEAS-6): `SimulizarSlidingWindow` with the
//! `KeepLastElementPriorToLowerBoundStrategy` and the `SlidingWindowUtilizationAggregator`,
//! whose arithmetic uses JScience `Amount` interval values (ported literally, [`Amount`]).

use std::collections::VecDeque;

/// `Amount.DECREMENT` / `INCREMENT`: `1 ∓ 2^-53` (`INCREMENT` rounds to exactly 1.0).
const DEC: f64 = 1.0 - 1.0 / 9_007_199_254_740_992.0;
const INC: f64 = 1.0 + 1.0 / 9_007_199_254_740_992.0;

/// JScience 4.3.1 `Amount` (unit handling omitted: all amounts here are seconds or ratios).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Amount {
    exact: Option<i64>,
    min: f64,
    max: f64,
}

#[inline]
fn adj(min: f64, max: f64) -> Amount {
    Amount {
        exact: None,
        min: if min < 0.0 { min * INC } else { min * DEC },
        max: if max < 0.0 { max * DEC } else { max * INC },
    }
}

impl Amount {
    /// `Amount.valueOf(double, unit)`.
    pub fn of(v: f64) -> Amount {
        let inc = v * INC;
        let dec = v * DEC;
        Amount {
            exact: None,
            min: if v < 0.0 { inc } else { dec },
            max: if v < 0.0 { dec } else { inc },
        }
    }

    /// `Amount.valueOf(long, unit)` (`setExact`).
    pub fn exact(l: i64) -> Amount {
        // setExact: `(double) l == (double) l` always holds in Java, so min = max = (double) l
        let d = l as f64;
        Amount {
            exact: Some(l),
            min: d,
            max: d,
        }
    }

    pub fn estimated(&self) -> f64 {
        match self.exact {
            Some(l) => l as f64,
            None => (self.min + self.max) * 0.5,
        }
    }

    pub fn plus(&self, o: &Amount) -> Amount {
        if let (Some(a), Some(b)) = (self.exact, o.exact) {
            let s = a.wrapping_add(b);
            if s as f64 == a as f64 + b as f64 {
                return Amount::exact(s);
            }
        }
        adj(self.min + o.min, self.max + o.max)
    }

    pub fn minus(&self, o: &Amount) -> Amount {
        if let (Some(a), Some(b)) = (self.exact, o.exact) {
            let s = a.wrapping_sub(b);
            if s as f64 == a as f64 - b as f64 {
                return Amount::exact(s);
            }
        }
        adj(self.min - o.max, self.max - o.min)
    }

    pub fn times_f64(&self, f: f64) -> Amount {
        let min = if f > 0.0 { self.min * f } else { self.max * f };
        let max = if f > 0.0 { self.max * f } else { self.min * f };
        adj(min, max)
    }

    fn times_long(&self, f: i64) -> Amount {
        if let Some(a) = self.exact {
            let p = a.wrapping_mul(f);
            if p as f64 == a as f64 * f as f64 {
                return Amount::exact(p);
            }
        }
        Amount {
            exact: None,
            min: if f > 0 {
                self.min * f as f64
            } else {
                self.max * f as f64
            },
            max: if f > 0 {
                self.max * f as f64
            } else {
                self.min * f as f64
            },
        }
    }

    pub fn times(&self, o: &Amount) -> Amount {
        if let Some(e) = o.exact {
            return self.times_long(e);
        }
        let (a, b) = (self, o);
        let (min, max);
        if a.min >= 0.0 {
            if b.min >= 0.0 {
                min = a.min * b.min;
                max = a.max * b.max;
            } else if b.max < 0.0 {
                min = a.max * b.min;
                max = a.min * b.max;
            } else {
                min = a.max * b.min;
                max = a.max * b.max;
            }
        } else if a.max < 0.0 {
            if b.min >= 0.0 {
                min = a.min * b.max;
                max = a.max * b.min;
            } else if b.max < 0.0 {
                min = a.max * b.max;
                max = a.min * b.min;
            } else {
                min = a.min * b.max;
                max = a.min * b.min;
            }
        } else if b.min >= 0.0 {
            min = a.min * b.max;
            max = a.max * b.max;
        } else if b.max < 0.0 {
            min = a.max * b.min;
            max = a.min * b.min;
        } else {
            min = (a.min * b.max).min(a.max * b.min);
            max = (a.min * b.min).max(a.max * b.max);
        }
        adj(min, max)
    }

    pub fn inverse(&self) -> Amount {
        if self.exact == Some(1) {
            return Amount::exact(1);
        }
        if self.min <= 0.0 && self.max >= 0.0 {
            return Amount {
                exact: None,
                min: f64::NEG_INFINITY,
                max: f64::INFINITY,
            };
        }
        adj(1.0 / self.max, 1.0 / self.min)
    }

    fn divide_long(&self, d: i64) -> Amount {
        if let Some(a) = self.exact
            && d != 0
        {
            let q = a.wrapping_div(d);
            if q as f64 == a as f64 / d as f64 {
                return Amount::exact(q);
            }
        }
        let min = if d > 0 {
            self.min / d as f64
        } else {
            self.max / d as f64
        };
        let max = if d > 0 {
            self.max / d as f64
        } else {
            self.min / d as f64
        };
        adj(min, max)
    }

    pub fn divide(&self, o: &Amount) -> Amount {
        if let Some(e) = o.exact {
            return self.divide_long(e);
        }
        self.times(&o.inverse())
    }

    /// `compareTo(that) > 0`.
    pub fn is_greater_than(&self, o: &Amount) -> bool {
        java_compare(self.estimated(), o.estimated()) > 0
    }

    /// `compareTo(that) < 0`: `Double.compare` of the estimated values.
    pub fn is_less_than(&self, o: &Amount) -> bool {
        java_compare(self.estimated(), o.estimated()) < 0
    }
}

/// `Double.compare`.
fn java_compare(a: f64, b: f64) -> i32 {
    if a < b {
        -1
    } else if a > b {
        1
    } else {
        let (x, y) = (java_bits(a), java_bits(b));
        (x > y) as i32 - (x < y) as i32
    }
}

fn java_bits(d: f64) -> i64 {
    if d.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        d.to_bits() as i64
    }
}

/// `SlidingWindowUtilizationAggregator.processWindowData`: `(right bound, utilisation)`.
pub fn utilization(data: &VecDeque<(f64, f64)>, lower: f64, length: f64) -> (f64, f64) {
    let l = Amount::of(lower);
    let w = Amount::of(length);
    let r = w.plus(&l);
    let mut busy = Amount::exact(0);
    let mut it = data.iter();
    if let Some(&(t0, v0)) = it.next() {
        let mut state = v0;
        let mut cur_t = Amount::of(t0);
        loop {
            if cur_t.is_less_than(&l) {
                cur_t = l;
            }
            let (next_t, next_v, end) = match it.next() {
                Some(&(t, v)) => (Amount::of(t), v, false),
                None => (r, 0.0, true),
            };
            busy = busy.plus(&next_t.minus(&cur_t).times_f64(state.min(1.0)));
            if end {
                break;
            }
            state = next_v;
            cur_t = next_t;
        }
    }
    let u = busy.divide(&w);
    (r.estimated(), u.estimated())
}

/// A `SimulizarSlidingWindow` (lower bound starts at 0).
#[derive(Clone, Debug)]
pub struct Window {
    pub len: f64,
    pub inc: f64,
    pub lower: f64,
    /// Accepted measurements `(point in time, value)`.
    pub data: VecDeque<(f64, f64)>,
    /// `true`: `KeepLastElementPriorToLowerBoundStrategy`; `false`: discard all prior elements.
    pub keep_last: bool,
}

impl Window {
    pub fn new(len: f64, inc: f64, keep_last: bool) -> Window {
        Window {
            len,
            inc,
            lower: 0.0,
            data: VecDeque::new(),
            keep_last,
        }
    }

    /// `addMeasurementInternal`: a measurement before the lower bound flushes the window first.
    pub fn add(&mut self, t: f64, v: f64) {
        if java_compare(self.lower, t) > 0 {
            self.data.clear();
        }
        self.data.push_back((t, v));
    }

    /// `getEffectiveWindowLength` at simulation time `now` (seconds).
    pub fn effective_length(&self, now: f64) -> f64 {
        let upper = (self.lower + self.len).min(now);
        upper - self.lower
    }

    /// `moveOn`: advance the lower bound and apply the move-on strategy.
    pub fn move_on(&mut self) {
        self.lower += self.inc;
        let lb = self.lower;
        let prior =
            |d: &VecDeque<(f64, f64)>| d.front().is_some_and(|&(t, _)| java_compare(t, lb) < 0);
        if prior(&self.data) {
            let mut first = self.data.pop_front().expect("nonempty");
            if self.keep_last {
                while prior(&self.data) {
                    first = self.data.pop_front().expect("nonempty");
                }
                self.data.push_front(first);
            } else {
                while prior(&self.data) {
                    self.data.pop_front();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn increment_rounds_to_one() {
        assert_eq!(INC, 1.0);
        assert_eq!(DEC, 1.0 - f64::EPSILON / 2.0);
    }

    #[test]
    fn jvm_checked_window_values() {
        // MEAS-6.5, executed on the JVM: window [0,10] busy from 2 s -> (9.999999999999998, 0.7999999999999996)
        let data: VecDeque<(f64, f64)> = [(2.0, 1.0)].into_iter().collect();
        let (t, u) = utilization(&data, 0.0, 10.0);
        assert_eq!(t, 9.999999999999998);
        assert_eq!(u, 0.7999999999999996);
        // right bound of [10,20]
        let (t, _) = utilization(&VecDeque::new(), 10.0, 10.0);
        assert_eq!(t, 19.999999999999996);
    }

    #[test]
    fn keep_last_element_prior_to_lower_bound() {
        let mut w = Window::new(10.0, 10.0, true);
        for (t, v) in [(1.0, 1.0), (4.0, 0.0), (12.0, 1.0)] {
            w.add(t, v);
        }
        w.move_on();
        assert_eq!(w.lower, 10.0);
        assert_eq!(w.data, VecDeque::from(vec![(4.0, 0.0), (12.0, 1.0)]));
        // a measurement before the lower bound flushes the data
        w.add(5.0, 1.0);
        assert_eq!(w.data, VecDeque::from(vec![(5.0, 1.0)]));
    }
}
