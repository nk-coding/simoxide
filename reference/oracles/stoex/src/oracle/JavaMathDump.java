package oracle;

import java.io.BufferedWriter;
import java.io.FileWriter;
import java.util.SplittableRandom;

/**
 * Dumps Math.pow / Math.log / StrictMath.pow / StrictMath.log and the double remainder (%) for a deterministic input set, as hex
 * bit patterns, to find out which Rust implementation reproduces the JVM that runs the reference.
 * Usage: JavaMathDump OUT N
 * Line format: op x y result_Math result_StrictMath   (all as 16-digit hex of the IEEE bits)
 */
public class JavaMathDump {
    public static void main(String[] args) throws Exception {
        int n = Integer.parseInt(args[1]);
        SplittableRandom r = new SplittableRandom(42);
        try (BufferedWriter w = new BufferedWriter(new FileWriter(args[0]))) {
            double[] specials = {0.0, -0.0, 1.0, -1.0, 2.0, 0.5, 10.0, Double.NaN, Double.POSITIVE_INFINITY,
                    Double.NEGATIVE_INFINITY, Double.MIN_VALUE, Double.MAX_VALUE, 1e-300, 1e300, 3.0, -2.0, -0.5};
            for (double x : specials) {
                for (double y : specials) {
                    pow(w, x, y);
                }
                log(w, x);
            }
            for (int i = 0; i < n; i++) {
                double x, y;
                switch (i % 6) {
                case 0: x = r.nextDouble() * 1000; y = r.nextInt(-10, 11); break;
                case 1: x = r.nextDouble() * 100; y = r.nextDouble() * 10 - 5; break;
                case 2: x = r.nextInt(-100, 101); y = r.nextInt(0, 20); break;
                case 3: x = Double.longBitsToDouble(r.nextLong() & 0x7fffffffffffffffL);
                        y = Double.longBitsToDouble(r.nextLong()); break;
                case 4: x = r.nextDouble() * 2; y = r.nextDouble() * 400 - 200; break;
                default: x = 1 + (r.nextDouble() - 0.5) * 1e-6; y = r.nextDouble() * 1e7; break;
                }
                pow(w, x, y);
                rem(w, x, y);
                log(w, x);
                log(w, r.nextDouble() * Math.pow(10, r.nextInt(-20, 20)));
            }
        }
    }

    static String h(double d) {
        return String.format("%016x", Double.doubleToRawLongBits(d));
    }

    static void pow(BufferedWriter w, double x, double y) throws Exception {
        w.write("pow " + h(x) + " " + h(y) + " " + h(Math.pow(x, y)) + " " + h(StrictMath.pow(x, y)) + "\n");
    }

    static void rem(BufferedWriter w, double x, double y) throws Exception {
        w.write("rem " + h(x) + " " + h(y) + " " + h(x % y) + " " + h(StrictMath.IEEEremainder(0, 1)) + "\n");
    }

    static void log(BufferedWriter w, double x) throws Exception {
        w.write("log " + h(x) + " 0000000000000000 " + h(Math.log(x)) + " " + h(StrictMath.log(x)) + "\n");
    }
}
