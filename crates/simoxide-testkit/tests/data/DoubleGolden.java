import java.util.*;
/** Prints "hexbits Double.toString" lines: edge cases, subnormals, powers, and random bit patterns. */
public class DoubleGolden {
    public static void main(String[] a) {
        long n = Long.parseLong(a[0]);
        long seed = Long.parseLong(a[1]);
        StringBuilder sb = new StringBuilder();
        List<Double> xs = new ArrayList<>();
        double[] edge = {0.0, -0.0, 1.0, -1.0, Double.MIN_VALUE, Double.MIN_NORMAL, Double.MAX_VALUE, 1e23, 1e22, 9007199254740993.0,
            2e-323, 1e-3, 0.0009999999999999998, 1e7, 9999999.999999998, 1e-4, 0.1, 0.2, 0.3, 100.0, 123456789.0, 5e-324, 1.0E-322, 2.5e-323};
        for (double d : edge) xs.add(d);
        for (long m = 1; m < 5000; m++) xs.add(Double.longBitsToDouble(m));            // smallest subnormals
        for (int e = -1074; e <= 1023; e++) { xs.add(Math.scalb(1.0, e)); xs.add(Math.nextDown(Math.scalb(1.0, e))); }
        for (int e = -325; e <= 308; e++) { double p = Double.parseDouble("1e" + e); xs.add(p); xs.add(Math.nextUp(p)); xs.add(Math.nextDown(p)); }
        SplittableRandom r = new SplittableRandom(seed);
        for (long i = 0; i < n; i++) {
            long bits = r.nextLong();
            double d = Double.longBitsToDouble(bits);
            if (Double.isNaN(d)) continue;
            xs.add(d);
            if ((i & 3) == 0) xs.add((double) (r.nextLong() >> r.nextInt(64)) / Math.pow(10, r.nextInt(12))); // decimal-ish
            if ((i & 7) == 0) xs.add(Double.longBitsToDouble(r.nextLong(1L << 52)));  // subnormal
        }
        java.io.PrintStream out = new java.io.PrintStream(new java.io.BufferedOutputStream(System.out, 1 << 16));
        for (double d : xs) {
            out.print(Long.toHexString(Double.doubleToRawLongBits(d)));
            out.print(' ');
            out.println(Double.toString(d));
        }
        out.flush();
    }
}
