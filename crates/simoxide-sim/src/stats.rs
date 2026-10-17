//! Statistical characterisations of runtime measurements (MEAS-7.5): the monitor repository's
//! `StatisticalCharacterizationAggregator`s (arithmetic, geometric and harmonic mean, median)
//! as SimuLizar 5.2.2 applies them to the data of a time-driven aggregation window or of a
//! fixed- or variable-size aggregation buffer before writing the result into the runtime
//! measurement model (PRM).
//!
//! Ported from the 5.2.2 jar `org.palladiosimulator.monitorrepository`
//! (`statisticalcharacterization/*Aggregator`, decompiled): discrete metrics average the values
//! with `Collectors.averagingDouble` (JDK 21's compensated sum, [`java_average`]); continuous
//! metrics weight each value with its validity length in JScience [`Amount`] arithmetic.

use crate::windows::Amount;
use simoxide_model::model::StatisticalCharacterization;

/// A statistical characterisation with the metric's scope of validity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stat {
    pub kind: StatKind,
    /// `ScopeOfValidity.CONTINUOUS` (else discrete).
    pub continuous: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatKind {
    ArithmeticMean,
    GeometricMean,
    HarmonicMean,
    Median,
}

impl StatKind {
    /// The model's characterisation (`None`: missing, which SimuLizar rejects when the
    /// aggregator is created).
    pub fn of(s: StatisticalCharacterization) -> Option<StatKind> {
        match s {
            StatisticalCharacterization::ArithmeticMean => Some(StatKind::ArithmeticMean),
            StatisticalCharacterization::GeometricMean => Some(StatKind::GeometricMean),
            StatisticalCharacterization::HarmonicMean => Some(StatKind::HarmonicMean),
            StatisticalCharacterization::Median => Some(StatKind::Median),
            StatisticalCharacterization::Missing => None,
        }
    }
}

/// `Collectors.averagingDouble` of JDK 21: Kahan-compensated sum (`sumWithCompensation`,
/// `computeFinalSum` subtracting the compensation) divided by the count; 0 for no values.
pub fn java_average(values: impl IntoIterator<Item = f64>) -> f64 {
    let (mut sum, mut comp, mut n, mut simple) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for v in values {
        let tmp = v - comp;
        let velvel = sum + tmp;
        comp = (velvel - sum) - tmp;
        sum = velvel;
        n += 1.0;
        simple += v;
    }
    if n == 0.0 {
        return 0.0;
    }
    let tmp = sum - comp;
    let total = if tmp.is_nan() && simple.is_infinite() {
        simple
    } else {
        tmp
    };
    total / n
}

/// `Double.compare(a, b) == 0`.
fn java_compare_zero(a: f64) -> bool {
    a.to_bits() == 0.0f64.to_bits()
}

/// `StatisticalCharacterizationAggregator.aggregateData`: the characterisation of `data`
/// (`(point in time, value)` in seconds and the metric's default unit, in the order given) over
/// the interval `[left, right]` of length `length`.
pub fn aggregate(
    stat: Stat,
    data: &[(f64, f64)],
    left: Amount,
    right: Amount,
    length: Amount,
) -> f64 {
    if !stat.continuous {
        let values = data.iter().map(|&(_, v)| v);
        return match stat.kind {
            StatKind::ArithmeticMean => java_average(values),
            StatKind::GeometricMean => {
                let m = java_average(values.map(simoxide_random::jmath::log));
                if java_compare_zero(m) || !m.is_finite() {
                    0.0
                } else {
                    simoxide_random::jmath::exp(m)
                }
            }
            StatKind::HarmonicMean => {
                let m = java_average(values.map(|v| 1.0 / v));
                // `m == Double.NaN` never holds in Java
                if m == 0.0 { 0.0 } else { 1.0 / m }
            }
            StatKind::Median => {
                let mut v: Vec<f64> = values.collect();
                // `Double.compareTo`: -0.0 before 0.0, NaN last
                v.sort_by(|a, b| java_cmp(*a, *b));
                if v.is_empty() {
                    0.0
                } else {
                    let mid = v.len() / 2;
                    if v.len().is_multiple_of(2) {
                        0.5 * (v[mid] + v[mid - 1])
                    } else {
                        v[mid]
                    }
                }
            }
        };
    }
    // continuous scope: each value holds from its point in time (at least the left bound) to
    // the next one's (the right bound for the last)
    let validity = |i: usize| -> Amount {
        let next = data.get(i + 1).map_or(right, |&(t, _)| Amount::of(t));
        let cur = Amount::of(data[i].0);
        next.minus(if cur.is_less_than(&left) { &left } else { &cur })
    };
    match stat.kind {
        StatKind::ArithmeticMean => {
            let mut area = Amount::of(0.0);
            for (i, &(_, v)) in data.iter().enumerate() {
                area = area.plus(&validity(i).times(&Amount::of(v)));
            }
            area.divide(&length).estimated()
        }
        StatKind::GeometricMean => {
            if data.is_empty() {
                return 0.0;
            }
            let mut sum_of_logs = 0.0f64;
            for (i, &(_, v)) in data.iter().enumerate() {
                sum_of_logs += simoxide_random::jmath::log(v) * validity(i).estimated();
            }
            if java_compare_zero(sum_of_logs) || !sum_of_logs.is_finite() {
                0.0
            } else {
                simoxide_random::jmath::exp(sum_of_logs / length.estimated())
            }
        }
        StatKind::HarmonicMean => {
            if data.is_empty() {
                return 0.0;
            }
            let mut area = Amount::of(0.0);
            for (i, &(_, v)) in data.iter().enumerate() {
                area = area.plus(&validity(i).divide(&Amount::of(v)));
            }
            length.divide(&area).estimated()
        }
        StatKind::Median => {
            if data.is_empty() {
                return 0.0;
            }
            // (length, value) intervals, stably sorted by value (`Amount.compareTo`)
            let mut iv: Vec<(Amount, Amount)> = (0..data.len())
                .map(|i| (validity(i), Amount::of(data[i].1)))
                .collect();
            iv.sort_by(|a, b| java_cmp(a.1.estimated(), b.1.estimated()));
            // prefix sums of the areas (`Arrays.parallelPrefix`, sequential for small arrays)
            let mut areas: Vec<Amount> = iv.iter().map(|(l, v)| l.times(v)).collect();
            for i in 1..areas.len() {
                areas[i] = areas[i - 1].plus(&areas[i]);
            }
            let half = areas[areas.len() - 1].divide_long(2);
            let idx = areas
                .iter()
                .position(|a| !a.is_less_than(&half))
                .unwrap_or(areas.len() - 1);
            iv[idx].1.estimated()
        }
    }
}

/// `Double.compare` as an ordering: `<`, `>`, else the signed comparison of the raw bits
/// (`-0.0` before `0.0`, NaN last).
fn java_cmp(a: f64, b: f64) -> std::cmp::Ordering {
    let bits = |d: f64| -> i64 {
        if d.is_nan() {
            0x7ff8_0000_0000_0000
        } else {
            d.to_bits() as i64
        }
    };
    if a < b {
        std::cmp::Ordering::Less
    } else if a > b {
        std::cmp::Ordering::Greater
    } else {
        bits(a).cmp(&bits(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_average_is_compensated() {
        // JDK 21 (checked on the JVM): averagingDouble subtracts the compensation
        assert_eq!(java_average([]), 0.0);
        assert_eq!(java_average([1.0, 2.0, 4.0]), 7.0 / 3.0);
        let v = [0.1, 0.2, 0.3];
        assert_eq!(java_average(v), 0.2);
        // plain summation would give 0.20000000000000004
        assert_ne!((0.1 + 0.2 + 0.3) / 3.0, 0.2);
    }

    #[test]
    fn discrete_statistics() {
        let s = |kind| Stat {
            kind,
            continuous: false,
        };
        let d = [(1.0, 1.0), (2.0, 4.0), (3.0, 16.0)];
        let (l, r) = (Amount::of(0.0), Amount::of(10.0));
        let a = aggregate(s(StatKind::ArithmeticMean), &d, l, r, r.minus(&l));
        assert_eq!(a, 7.0);
        let g = aggregate(s(StatKind::GeometricMean), &d, l, r, r.minus(&l));
        assert!((g - 4.0).abs() < 1e-12);
        let h = aggregate(s(StatKind::HarmonicMean), &d, l, r, r.minus(&l));
        assert!((h - 3.0 / (1.0 + 0.25 + 0.0625)).abs() < 1e-12);
        assert_eq!(aggregate(s(StatKind::Median), &d, l, r, r.minus(&l)), 4.0);
        assert_eq!(
            aggregate(s(StatKind::Median), &d[..2], l, r, r.minus(&l)),
            2.5
        );
        assert_eq!(
            aggregate(s(StatKind::ArithmeticMean), &[], l, r, r.minus(&l)),
            0.0
        );
    }

    #[test]
    fn continuous_mean_weights_by_validity() {
        let s = Stat {
            kind: StatKind::ArithmeticMean,
            continuous: true,
        };
        // 0 from 0 to 5 (the first value lies before the window), 2 from 5 to 10
        let d = [(-1.0, 0.0), (5.0, 2.0)];
        let (l, r) = (Amount::of(0.0), Amount::of(10.0));
        let m = aggregate(s, &d, l, r, Amount::of(10.0));
        assert!((m - 1.0).abs() < 1e-12);
        let med = aggregate(
            Stat {
                kind: StatKind::Median,
                continuous: true,
            },
            &d,
            l,
            r,
            Amount::of(10.0),
        );
        assert_eq!(med, 2.0);
    }
}
