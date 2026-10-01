// SimOxide: Rust translation of the Intel LIBM stubs of OpenJDK HotSpot,
// src/hotspot/cpu/x86/stubGenerator_x86_64_log.cpp and stubGenerator_x86_64_exp.cpp (jdk21u).
// Translated by the SimOxide authors in September 2026 (assembler instructions rewritten as
// Rust scalar operations, tables converted to u64 words). The original files carry this notice:
//
// Copyright (c) 2016, 2021, Intel Corporation. All rights reserved.
// Copyright (C) 2021, Tencent. All rights reserved.
// Intel Math Library (LIBM) Source Code
//
// This code is free software; you can redistribute it and/or modify it
// under the terms of the GNU General Public License version 2 only, as
// published by the Free Software Foundation.
//
// This code is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or
// FITNESS FOR A PARTICULAR PURPOSE.  See the GNU General Public License
// version 2 for more details (a copy is included in the LICENSE file that
// accompanied this code).
//
// You should have received a copy of the GNU General Public License version
// 2 along with this work; if not, write to the Free Software Foundation,
// Inc., 51 Franklin St, Fifth Floor, Boston, MA 02110-1301 USA.

//! `java.lang.Math.log` and `Math.exp` exactly as HotSpot computes them on x86-64.
//!
//! On x86-64, `Math.log` and `Math.exp` are intrinsics backed by the Intel LIBM stubs
//! (jdk21u `src/hotspot/cpu/x86/stubGenerator_x86_64_{log,exp}.cpp`, identical since JDK 9),
//! used by the interpreter, C1 and C2 alike. They are neither fdlibm (`StrictMath`) nor the C
//! library, and not always correctly rounded. This crate ports both stubs instruction by
//! instruction (the SSE2 lane operations are written out as scalar operations in the same order),
//! so the results are bit-identical.
//!
//! The only hardware-dependent step is `rcpps` (approximate reciprocal) in `log`, whose exact
//! result differs between CPU vendors. On x86-64 we execute the same instruction, so `log` matches
//! a JVM running on the same machine. On other targets, or with the `rcp-table` feature, a
//! recorded table (AMD Zen 5) is used.
//!
//! **Licence.** The stubs are licensed under GPL-2.0-only (without the Classpath Exception), so
//! this crate is too, unlike the rest of SimOxide (EPL-2.0). `simoxide-random` uses it only with
//! its feature `hotspot-math` (off by default in the libraries, on in the command line tools);
//! binaries built with that feature contain GPL-2.0-only code together with EPL-2.0 and
//! Apache-2.0 code, whose licences are incompatible with it, and must not be distributed. See
//! `LICENSES` in the repository root.

mod rcp;
mod tables;

use tables::{EXP_CV, EXP_TBL, LOG_COEFF, LOG_LOG2, LOG_TBL};

#[inline(always)]
fn d(bits: u64) -> f64 {
    f64::from_bits(bits)
}

/// `(rcpps(lane) + 0x8000) >> 16` for a float `lane` in `[1, 2)`: the reciprocal rounded to
/// 7 mantissa bits, as sign/exponent/mantissa bits 16..30 of the float.
#[inline(always)]
fn rcp_rounded(lane: u32) -> u32 {
    (rcp_bits(lane).wrapping_add(0x8000)) >> 16
}

#[cfg(all(target_arch = "x86_64", not(feature = "rcp-table")))]
#[inline(always)]
fn rcp_bits(lane: u32) -> u32 {
    use core::arch::x86_64::{_mm_cvtss_f32, _mm_rcp_ss, _mm_set_ss};
    // SAFETY: these intrinsics only require SSE, which is part of the x86-64 baseline and thus
    // always available; they have no memory effects. (Rust still requires `unsafe` because the
    // calling function is not itself `#[target_feature(enable = "sse")]`.) Covered by
    // `tests::rcp_table_matches_hardware` and the golden-file tests.
    unsafe { _mm_cvtss_f32(_mm_rcp_ss(_mm_set_ss(f32::from_bits(lane)))).to_bits() }
}

#[cfg(any(not(target_arch = "x86_64"), feature = "rcp-table"))]
#[inline(always)]
fn rcp_bits(lane: u32) -> u32 {
    // Recorded behaviour of `rcpss` on AMD Zen 5 (see `RCP_BREAKS`): only bits 16..30 of
    // `rcp + 0x8000` matter, so return a representative value with those bits.
    rcp_rounded_table(lane) << 16
}

/// Breakpoints of `rcp_rounded` over float inputs `[1, 2)` as recorded on AMD Zen 5:
/// `(first input bits, rounded value)`, sorted by input.
#[allow(dead_code)]
pub(crate) const RCP_BREAKS: &[(u32, u32)] = &rcp::RCP_BREAKS;

#[allow(dead_code)]
pub(crate) fn rcp_rounded_table(lane: u32) -> u32 {
    let i = RCP_BREAKS.partition_point(|&(start, _)| start <= lane);
    RCP_BREAKS[i.saturating_sub(1)].1
}

/// `Math.log(x)` (HotSpot x86-64 intrinsic `libmLog`).
pub fn log(x: f64) -> f64 {
    let bits = x.to_bits();
    let hi16 = (bits >> 48) as u32;
    let eax = hi16.wrapping_sub(16);
    if eax >= 32736 {
        return log_special(x, bits, hi16);
    }
    log_main(bits, eax, 16352)
}

/// The main path (label `L_2TAG_PACKET_1_0_2`), shared with the subnormal path.
/// `eax` is the (adjusted) top 16 bits, `ecx` the exponent bias in the same format.
#[inline(always)]
fn log_main(bits: u64, eax: u32, ecx: u32) -> f64 {
    // xmm0 = (x | 1.0) >> 27 (qword), >> 2 (dwords): lane 0 = top 23 mantissa bits as a float
    // in [1, 2). Lane 1 is a denormal whose reciprocal is +inf (low bits 0), so bits 61..63 of
    // the shifted qword below are zero.
    let t = (bits | 0x3ff0_0000_0000_0000) >> 27;
    let lane0 = (t as u32) >> 2;
    let bq = rcp_rounded(lane0); // (lane0' + 0x8000) >> 16
    let edx = bq << 16; // only bits 16..23 are used below
    let b = d((bq as u64) << 45); // psllq 29 of (bq << 16), masked with 0xffffe000_00000000
    let m = d(((bits << 12) >> 12) | 0x77f0_0000_0000_0000);
    let mhi = d(m.to_bits() & 0xffff_e000_0000_0000);
    let mlo = m - mhi;
    let mut x5 = mhi * b;
    let k = (((eax & 32752) as i32) - ecx as i32) as f64;
    let mut x1 = mlo * b;
    let mut x6 = d(LOG_LOG2[0]);
    let x3_lo = d(LOG_COEFF[0]);
    let x3_hi = d(LOG_COEFF[1]);
    x5 -= 1.0;
    let off = (((edx & 16_711_680) >> 12) / 8) as usize;
    let t_hi = d(LOG_TBL[off]);
    let t_lo = d(LOG_TBL[off + 1]);
    let x4_lo = d(LOG_COEFF[2]);
    let x4_hi = d(LOG_COEFF[3]);
    x1 += x5; // r
    let r = x1;
    let x2_lo = d(LOG_COEFF[4]);
    let x2_hi = d(LOG_COEFF[5]);
    x6 *= k;
    let mut x7 = k * d(LOG_LOG2[1]);
    let x3_lo = x3_lo * r;
    let mut x0 = t_hi + x6; // A
    let x4_lo = x4_lo * r;
    let x4_hi = x4_hi * r;
    let r2 = r * r;
    let a = x0;
    x0 += x1; // A + r
    let x4_lo = x4_lo + x2_lo;
    let x4_hi = x4_hi + x2_hi;
    let x3_lo = x3_lo * r2;
    let x3_hi = x3_hi * r2;
    let x6 = a - x0;
    let x4_lo = x4_lo * x1;
    x1 += x6;
    let r4 = r2 * r2;
    x7 += t_lo;
    let x4_lo = x4_lo + x3_lo;
    let x4_hi = x4_hi + x3_hi;
    x1 += x7;
    let x4_lo = x4_lo * r4;
    let x4_hi = x4_hi * r2;
    x1 += x4_lo;
    x1 += x4_hi;
    x0 + x1
}

#[cold]
fn log_special(x: f64, bits: u64, hi16: u32) -> f64 {
    if hi16 >= 32768 {
        // Negative (sign bit set), including -0, -inf and NaNs with the sign bit.
        let lo = bits as u32;
        let hi2 = ((bits >> 32) as u32).wrapping_add((bits >> 32) as u32);
        if hi2 >= 0xffe0_0000 {
            if hi2 > 0xffe0_0000 || lo > 0 {
                return x + x; // NaN
            }
            return 0.0 * f64::INFINITY; // -inf -> NaN
        }
        if (lo | hi2) == 0 {
            return -1.0 / 0.0; // -0.0 -> -inf
        }
        return 0.0 * f64::INFINITY; // negative -> NaN
    }
    if hi16 < 16 {
        // +0 or subnormal.
        if bits == 0 {
            return -1.0 / 0.0;
        }
        let xs = x * d(0x47f0_0000_0000_0000); // * 2^128
        let sbits = xs.to_bits();
        let eax = (sbits >> 48) as u32; // no "- 16" on this path
        return log_main(sbits, eax, 18416);
    }
    // +inf or NaN.
    x + x
}

/// `Math.exp(x)` (HotSpot x86-64 intrinsic `libmExp`).
pub fn exp(x: f64) -> f64 {
    let bits = x.to_bits();
    let ax = ((bits >> 48) as u32) & 32767;
    let edx0 = 16527u32.wrapping_sub(ax) | ax.wrapping_sub(15504);
    if edx0 >= 0x8000_0000 {
        return exp_special(x, bits);
    }
    let l2e = d(EXP_CV[0]);
    let ln2hi = d(EXP_CV[2]);
    let ln2lo = d(EXP_CV[4]);
    let shifter = d(0x4338_0000_0000_0000);
    let mut x1 = l2e * x;
    x1 += shifter;
    let x7bits = x1.to_bits();
    x1 -= shifter; // N
    let x2 = ln2hi * x1;
    let x4_lo_c = d(EXP_CV[8]);
    let x4_hi_c = d(EXP_CV[9]);
    let x3 = ln2lo * x1;
    let x5_lo_c = d(EXP_CV[10]);
    let x5_hi_c = d(EXP_CV[11]);
    let mut x0 = x - x2;
    let eax_n = x7bits as u32;
    let ecx = ((eax_n & 63) << 4) as usize;
    let n = (eax_n as i32) >> 6;
    // xmm7 = (((N & ~63) + 0xffc0) << 46): 2^n (exponent field only)
    let x7 = ((x7bits & 0xffff_ffc0) + 0xffc0) << 46; // bits above 63 are dropped, as in psllq
    x0 -= x3; // r
    let t_lo = d(EXP_TBL[ecx / 8]);
    let t_hi_bits = EXP_TBL[ecx / 8 + 1];
    let x4_lo = x4_lo_c * x0;
    let x4_hi = x4_hi_c * x0;
    let mut x1 = x0;
    let r2 = x0 * x0;
    let x0_lo = x0 * r2; // r^3
    let x0_hi = x0 * r2;
    let x5_lo = x5_lo_c + x4_lo;
    let x5_hi = x5_hi_c + x4_hi;
    let x0_lo = x0_lo * r2; // r^5
    let x6 = r2 * d(EXP_CV[6]);
    x1 += t_lo;
    let x0_lo = x0_lo * x5_lo;
    let x0_hi = x0_hi * x5_hi;
    x1 += x0_lo;
    let t = d(t_hi_bits | x7);
    let mut p = x0_hi + x1;
    p += x6;
    let edx = (n as u32).wrapping_add(894);
    if edx > 1916 {
        return exp_scale(p, t, n);
    }
    p *= t;
    p + t
}

/// Label `L_2TAG_PACKET_1_0_2`: results near overflow or underflow.
#[cold]
fn exp_scale(p: f64, t: f64, n: i32) -> f64 {
    let edx = (-1022i32).wrapping_sub(n);
    // psllq(ALLONES, edx): count is the zero-extended 32-bit value; >= 64 gives 0.
    let count = edx as u32 as u64;
    let x4_mask = if count > 63 { 0 } else { u64::MAX << count };
    let half = n >> 1;
    let x3 = (((half as u32) & 0xffff) as u64) << 52; // pinsrw(.., 3) then psllq 4
    // psubd: per 32-bit lane, the low lane of x3 is zero.
    let t_bits = t.to_bits();
    let t_hi = ((t_bits >> 32) as u32).wrapping_sub((x3 >> 32) as u32);
    let x2 = d(((t_hi as u64) << 32) | (t_bits & 0xffff_ffff));
    let mut x0 = p * x2;
    if edx > 52 {
        // L2: total underflow region.
        let x3v = d(add_dwords(x3, 0x3ff0_0000_0000_0000));
        x0 += x2;
        return x0 * x3v;
    }
    let x4 = d(x4_mask & x2.to_bits());
    let x3v = d(add_dwords(x3, 0x3ff0_0000_0000_0000));
    let x2b = x2 - x4;
    x0 += x2b;
    if n >= 1023 {
        // L3: overflow check (errno only); value as computed.
        return (x0 + x4) * x3v;
    }
    let sign = ((x0.to_bits() >> 48) as u32) & 32768;
    if (edx as u32 | sign) == 0 {
        // L4
        return (x0 + x4) * x3v;
    }
    let x6 = x0;
    let r = (x0 + x4) * x3v;
    if ((r.to_bits() >> 48) as u32) & 32752 != 0 {
        return r;
    }
    // L5: subnormal result, assembled with integer arithmetic.
    let x6 = (x6 * x3v).to_bits();
    let x4 = (x4 * x3v).to_bits();
    let mut x0 = x6;
    let s = ((((x6 ^ x4) >> 32) as u32 as i32) >> 31) as u32; // psrad 31 of the high dword
    let s64 = ((s as u64) << 32) | s as u64; // pshufd 85 broadcasts dword 1
    x0 = (x0 << 1) >> 1;
    x0 ^= s64;
    x0 = x0.wrapping_add(s64 >> 63);
    x0 = x0.wrapping_add(x4);
    d(x0)
}

#[inline(always)]
fn add_dwords(a: u64, b: u64) -> u64 {
    let lo = (a as u32).wrapping_add(b as u32);
    let hi = ((a >> 32) as u32).wrapping_add((b >> 32) as u32);
    ((hi as u64) << 32) | lo as u64
}

#[cold]
fn exp_special(x: f64, bits: u64) -> f64 {
    let hi = (bits >> 32) as u32;
    let ahi = hi & 0x7fff_ffff;
    if ahi < 1_083_179_008 {
        // |x| < 2^-54
        return x + 1.0;
    }
    if ahi >= 2_146_435_072 {
        // inf or NaN
        let lo = bits as u32;
        if ahi > 2_146_435_072 || lo != 0 {
            return x + x;
        }
        if hi != 2_146_435_072 {
            return 0.0; // -inf
        }
        return f64::INFINITY;
    }
    if hi >= 0x8000_0000 {
        let m = d(0x0010_0000_0000_0000);
        return m * m; // underflow to +0
    }
    let m = d(0x7fef_ffff_ffff_ffff);
    m * m // overflow to +inf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_basics() {
        assert_eq!(log(1.0), 0.0);
        assert_eq!(log(0.0), f64::NEG_INFINITY);
        assert_eq!(log(-0.0), f64::NEG_INFINITY);
        assert!(log(-1.0).is_nan());
        assert_eq!(log(f64::INFINITY), f64::INFINITY);
        assert!(log(f64::NAN).is_nan());
        for &x in &[
            2.0, 0.5, 10.0, 1e-300, 1e300, 5e-324, 3.3e-310, 0.999, 1.001,
        ] {
            let (a, b) = (log(x), x.ln());
            assert!((a - b).abs() <= b.abs() * 4e-16, "log({x}) = {a} vs {b}");
        }
    }

    #[test]
    fn exp_basics() {
        assert_eq!(exp(0.0), 1.0);
        assert_eq!(exp(f64::NEG_INFINITY), 0.0);
        assert_eq!(exp(f64::INFINITY), f64::INFINITY);
        assert_eq!(exp(1000.0), f64::INFINITY);
        assert_eq!(exp(-1000.0), 0.0);
        assert!(exp(f64::NAN).is_nan());
        for &x in &[
            1.0, -1.0, 0.5, 10.0, -700.0, 700.0, -740.0, -708.5, 1e-10, 709.7,
        ] {
            let (a, b) = (exp(x), x.exp());
            assert!(
                (a - b).abs() <= b.abs() * 4e-16 + 1e-322,
                "exp({x}) = {a} vs {b}"
            );
        }
    }

    #[test]
    fn rcp_table_matches_hardware() {
        // Documents on which CPUs the recorded table (used off x86-64) is exact.
        let mut diff = 0u32;
        for lane in (0x3f80_0000u32..0x4000_0000).step_by(7) {
            if rcp_rounded(lane) != rcp_rounded_table(lane) {
                diff += 1;
            }
        }
        if diff != 0 {
            eprintln!("rcpss on this CPU differs from the recorded Zen 5 table at {diff} inputs");
        }
    }
}
