package randomoracle;

import java.io.BufferedWriter;
import java.io.FileWriter;
import java.io.IOException;
import java.io.PrintWriter;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

import de.uka.ipd.sdq.probfunction.BoxedPDF;
import de.uka.ipd.sdq.probfunction.ContinuousSample;
import de.uka.ipd.sdq.probfunction.ProbabilityMassFunction;
import de.uka.ipd.sdq.probfunction.ProbfunctionFactory;
import de.uka.ipd.sdq.probfunction.Sample;
import de.uka.ipd.sdq.probfunction.math.IProbabilityDensityFunction;
import de.uka.ipd.sdq.probfunction.math.IProbabilityMassFunction;
import de.uka.ipd.sdq.probfunction.math.IRandomGenerator;
import de.uka.ipd.sdq.probfunction.math.apache.impl.MT19937RandomGenerator;
import de.uka.ipd.sdq.probfunction.math.apache.impl.PDFFactory;
import de.uka.ipd.sdq.probfunction.math.impl.ProbabilityFunctionFactoryImpl;
import de.uka.ipd.sdq.simucomframework.core.SimuComDefaultRandomNumberGenerator;
import de.uka.ipd.sdq.simucomframework.variables.cache.ProbFunctionCache;
import de.uka.ipd.sdq.simucomframework.variables.functions.ExpDistFunction;
import de.uka.ipd.sdq.simucomframework.variables.functions.GammaDistFunction;
import de.uka.ipd.sdq.simucomframework.variables.functions.GammaDistFunctionFromMoments;
import de.uka.ipd.sdq.simucomframework.variables.functions.IFunction;
import de.uka.ipd.sdq.simucomframework.variables.functions.LogNormDistFunction;
import de.uka.ipd.sdq.simucomframework.variables.functions.LogNormDistFunctionFromMoments;
import de.uka.ipd.sdq.simucomframework.variables.functions.NormDistFunction;
import de.uka.ipd.sdq.simucomframework.variables.functions.PoissonDistFunction;
import de.uka.ipd.sdq.simucomframework.variables.functions.UniDoubleDistFunction;
import de.uka.ipd.sdq.simucomframework.variables.functions.UniIntDistFunction;
import de.uka.ipd.sdq.stoex.ProbabilityFunctionLiteral;
import de.uka.ipd.sdq.stoex.StoexFactory;

/**
 * Golden-file generator for crates/simoxide-random. Calls the real SimuLizar 5.2.2 classes.
 *
 * <pre>
 * uniforms <out> <n> <s0,..,s5>   SimuComDefaultRandomNumberGenerator stream
 * mathfns  <out> <n> <seed>       Math.log / Math.exp / StrictMath.log|exp on many inputs
 * dists    <out> <n> <seed>       every StoEx distribution function x parameter grid (+ fixed-u cases)
 * probfn   <out> <n> <seed>       PMF / boxed PDF literals through ProbFunctionCache
 * </pre>
 *
 * Doubles are written as 16 hex digits (Double.doubleToRawLongBits).
 */
public class Main {

    static String hex(double d) {
        return String.format("%016x", Double.doubleToRawLongBits(d));
    }

    /** Records every uniform drawn; delegates to the product's MT19937 or to a fixed list. */
    static final class RecordingRng implements IRandomGenerator {
        final MT19937RandomGenerator mt;
        final double[] fixed;
        int fixedPos = 0;
        final List<Double> drawn = new ArrayList<>();

        RecordingRng(long seed, double[] fixed) {
            this.fixed = fixed;
            if (fixed == null) {
                mt = new MT19937RandomGenerator();
                mt.setSeed(new long[] { seed, seed + 1, seed + 2, seed + 3, seed + 4, seed + 5 });
            } else {
                mt = null;
            }
        }

        @Override
        public double random() {
            double u = fixed != null ? fixed[fixedPos++ % fixed.length] : mt.nextDouble();
            drawn.add(u);
            return u;
        }

        @Override
        public void dispose() {
        }
    }

    static PrintWriter open(String path) throws IOException {
        return new PrintWriter(new BufferedWriter(new FileWriter(path), 1 << 20));
    }

    public static void main(String[] args) throws Exception {
        switch (args[0]) {
        case "uniforms":
            uniforms(args[1], Integer.parseInt(args[2]), args[3]);
            break;
        case "mathfns":
            mathfns(args[1], Integer.parseInt(args[2]), Long.parseLong(args[3]));
            break;
        case "dists":
            dists(args[1], Integer.parseInt(args[2]), Long.parseLong(args[3]));
            break;
        case "probfn":
            probfn(args[1], Integer.parseInt(args[2]), Long.parseLong(args[3]));
            break;
        default:
            throw new IllegalArgumentException(args[0]);
        }
        System.exit(0); // SimuComDefaultRandomNumberGenerator leaves a producer thread running
    }

    static void uniforms(String out, int n, String seedCsv) throws IOException {
        String[] parts = seedCsv.split(",");
        long[] seed = new long[6];
        for (int i = 0; i < 6; i++) {
            seed[i] = Long.parseLong(parts[i]);
        }
        SimuComDefaultRandomNumberGenerator rng = new SimuComDefaultRandomNumberGenerator(seed);
        try (PrintWriter w = open(out)) {
            w.println("# uniforms seed=" + seedCsv + " n=" + n);
            for (int i = 0; i < n; i++) {
                w.println(hex(rng.random()));
            }
        }
        rng.dispose();
    }

    // ------------------------------------------------------------------------------------------

    static void mathfns(String out, int n, long seed) throws IOException {
        MT19937RandomGenerator mt = new MT19937RandomGenerator();
        mt.setSeed(new long[] { seed, 0, 0, 0, 0, 1 });
        java.util.Random jr = new java.util.Random(seed);
        List<Double> logIn = new ArrayList<>();
        List<Double> expIn = new ArrayList<>();
        double[] specials = { 0.0, -0.0, 1.0, -1.0, 2.0, 0.5, Double.MIN_VALUE, Double.MIN_NORMAL,
                Double.MAX_VALUE, Double.POSITIVE_INFINITY, Double.NEGATIVE_INFINITY, Double.NaN,
                Math.E, 1e-300, 1e300, 4.9e-324 * 3, Double.MIN_NORMAL / 3, 0x1.fffffffffffffp-1,
                0x1.0000000000001p0, 709.782712893384, 709.79, -745.1332191019412, -745.14, -708.4,
                -708.39, 1e-20, -1e-20, 0x1p-1022, 0x1p-60, 0.6931471805599453, -0.6931471805599453 };
        for (double s : specials) {
            logIn.add(s);
            expIn.add(s);
        }
        for (int i = 0; i < n; i++) {
            double u = mt.nextDouble();
            logIn.add(1.0 - u); // Exp inversion argument
            logIn.add(u); // regularizedGamma / lognormal arguments
            logIn.add(Double.longBitsToDouble(jr.nextLong() & 0x7fffffffffffffffL)); // any positive
            logIn.add(1.0 + (jr.nextInt(2001) - 1000) * 0x1p-52); // around 1
            logIn.add(1.0 + (jr.nextDouble() - 0.5) * 0x1p-5); // |x-1| small
            logIn.add(Math.scalb(1.0 + jr.nextDouble(), jr.nextInt(2100) - 1075)); // wide range
            expIn.add((jr.nextDouble() * 2 - 1) * 750.0);
            expIn.add((jr.nextDouble() * 2 - 1) * 40.0);
            expIn.add((jr.nextDouble() * 2 - 1) * Math.scalb(1.0, -jr.nextInt(80)));
            expIn.add(Double.longBitsToDouble(jr.nextLong()));
            expIn.add(-u * 745.2);
        }
        double[] li = logIn.stream().mapToDouble(Double::doubleValue).toArray();
        double[] ei = expIn.stream().mapToDouble(Double::doubleValue).toArray();
        double[] lo = new double[li.length];
        double[] so = new double[li.length];
        double[] eo = new double[ei.length];
        double[] seo = new double[ei.length];
        // Warm up so that the loops below run compiled (C2) code as in a long simulation;
        // the interpreter uses the same stubs, which the second pass cross-checks.
        for (int rep = 0; rep < 3; rep++) {
            for (int i = 0; i < li.length; i++) {
                double r = Math.log(li[i]);
                if (rep > 0 && Double.doubleToRawLongBits(r) != Double.doubleToRawLongBits(lo[i])) {
                    throw new IllegalStateException("Math.log not stable at " + li[i]);
                }
                lo[i] = r;
                so[i] = StrictMath.log(li[i]);
            }
            for (int i = 0; i < ei.length; i++) {
                double r = Math.exp(ei[i]);
                if (rep > 0 && Double.doubleToRawLongBits(r) != Double.doubleToRawLongBits(eo[i])) {
                    throw new IllegalStateException("Math.exp not stable at " + ei[i]);
                }
                eo[i] = r;
                seo[i] = StrictMath.exp(ei[i]);
            }
        }
        try (PrintWriter w = open(out)) {
            w.println("# mathfns: log <x> <Math.log> <StrictMath.log> | exp <x> <Math.exp> <StrictMath.exp>");
            w.println("# java " + System.getProperty("java.version") + " " + System.getProperty("os.arch"));
            for (int i = 0; i < li.length; i++) {
                w.println("log " + hex(li[i]) + " " + hex(lo[i]) + " " + hex(so[i]));
            }
            for (int i = 0; i < ei.length; i++) {
                w.println("exp " + hex(ei[i]) + " " + hex(eo[i]) + " " + hex(seo[i]));
            }
        }
    }

    // ------------------------------------------------------------------------------------------

    static final double[] FIXED_U = { 0.0, 0x1p-52, 0x1p-40, 1e-10, 1e-5, 0.001, 0.1, 0.25, 0.3, 0.5,
            0.5 + 0x1p-52, 0.5 - 0x1p-53, 0.7, 0.75, 0.9, 0.999, 1 - 1e-5, 1 - 1e-10, 1 - 0x1p-40,
            1 - 0x1p-52 };

    interface FnFactory {
        IFunction make(IRandomGenerator rng, PDFFactory f);
    }

    static final class Case {
        final String fn;
        final Object[] params;
        final FnFactory factory;

        Case(String fn, FnFactory factory, Object... params) {
            this.fn = fn;
            this.factory = factory;
            this.params = params;
        }

        String paramString() {
            StringBuilder sb = new StringBuilder();
            for (Object p : params) {
                if (sb.length() > 0) {
                    sb.append(' ');
                }
                if (p instanceof Integer) {
                    sb.append("i:").append(p);
                } else {
                    sb.append("d:").append(hex((Double) p));
                }
            }
            return sb.toString();
        }
    }

    static List<Case> cases() {
        List<Case> c = new ArrayList<>();
        FnFactory exp = ExpDistFunction::new;
        for (double r : new double[] { 1e-12, 0.001, 0.5, 1, 2, 7.3, 1000, 1e12, 1e300, 0, -1,
                Double.POSITIVE_INFINITY }) {
            c.add(new Case("Exp", exp, r));
        }
        FnFactory norm = NormDistFunction::new;
        for (double[] p : new double[][] { { 0, 1 }, { 10, 2 }, { -5, 0.1 }, { 0, 1e-9 }, { 1e6, 1 },
                { 3.5, 100 }, { 100, 30 }, { 0, 0 }, { 0, -1 } }) {
            c.add(new Case("Norm", norm, p[0], p[1]));
        }
        FnFactory lognorm = LogNormDistFunction::new;
        for (double[] p : new double[][] { { 0, 1 }, { 1, 0.5 }, { -2, 2 }, { 5, 0.1 }, { 0, 3 },
                { 2.3, 1.2 }, { 0, 0 } }) {
            c.add(new Case("Lognorm", lognorm, p[0], p[1]));
        }
        FnFactory lognormM = LogNormDistFunctionFromMoments::new;
        for (double[] p : new double[][] { { 1, 1 }, { 10, 2 }, { 0.5, 0.1 }, { 100, 300 }, { 3, 0.5 },
                { 0, 1 }, { 1, 0 } }) {
            c.add(new Case("LognormMoments", lognormM, p[0], p[1]));
        }
        FnFactory gamma = GammaDistFunction::new;
        for (double[] p : new double[][] { { 1, 1 }, { 2, 0.5 }, { 0.5, 2 }, { 9, 0.3 }, { 0.1, 1 },
                { 30, 1 }, { 3, 10 }, { 0, 1 } }) {
            c.add(new Case("Gamma", gamma, p[0], p[1]));
        }
        FnFactory gammaM = GammaDistFunctionFromMoments::new;
        for (double[] p : new double[][] { { 1, 1 }, { 5, 0.5 }, { 0.2, 2 }, { 10, 0.1 }, { 2, 1.5 },
                { 0, 1 }, { 1, 0 } }) {
            c.add(new Case("GammaMoments", gammaM, p[0], p[1]));
        }
        FnFactory pois = PoissonDistFunction::new;
        for (double m : new double[] { 0.5, 1, 4, 10, 30, 100, 1e-6, 1000, 0, -1 }) {
            c.add(new Case("Pois", pois, m));
        }
        FnFactory unid = UniDoubleDistFunction::new;
        for (double[] p : new double[][] { { 0, 1 }, { -3, 7 }, { 1e-9, 2e-9 }, { 100, 1e6 }, { 2.5, 2.6 },
                { -1e-3, 0 }, { 5, 5 }, { 2, 1 } }) {
            c.add(new Case("UniDouble", unid, p[0], p[1]));
        }
        FnFactory unii = UniIntDistFunction::new;
        for (int[] p : new int[][] { { 0, 1 }, { 1, 6 }, { -10, 10 }, { 5, 5 }, { 0, 1000000 },
                { -1000000000, 1000000000 }, { -2147483647, 0 }, { Integer.MIN_VALUE, Integer.MAX_VALUE }, { 3, 2 } }) {
            c.add(new Case("UniInt", unii, p[0], p[1]));
        }
        return c;
    }

    static String result(Object r) {
        if (r instanceof Integer) {
            return "I:" + r;
        }
        if (r instanceof Double) {
            return "D:" + hex((Double) r);
        }
        return "O:" + r;
    }

    static String error(Throwable t) {
        return "E:" + t.getClass().getSimpleName();
    }

    static void dists(String out, int n, long seed) throws IOException {
        try (PrintWriter w = open(out)) {
            w.println("# dists: case <fn> <params>; then per call: <uniforms consumed, comma-separated or -> <result>");
            int id = 0;
            for (Case c : cases()) {
                for (int mode = 0; mode < 2; mode++) {
                    // Lognormal inversion brackets linearly from exp(mu +- sigma) in steps of 1.0: the
                    // extreme upper tail takes ~exp(quantile) CDF evaluations (Lognorm(0,3) at
                    // u = 1-1e-10: ~1e8), so the committed goldens skip it for heavy tails.
                    double[] fixed = mode == 0 ? null
                            : c.fn.startsWith("Lognorm") ? Arrays.copyOf(FIXED_U, FIXED_U.length - 3) : FIXED_U;
                    int count = mode == 0 ? n : fixed.length;
                    RecordingRng rng = new RecordingRng(seed + id, fixed);
                    PDFFactory f = new PDFFactory();
                    f.setRandomGenerator(rng);
                    IFunction fn = c.factory.make(rng, f);
                    List<Object> params = Arrays.asList(c.params);
                    w.println("case " + (mode == 0 ? "seed=" + (seed + id) : "fixed") + " " + c.fn + " "
                            + c.paramString());
                    boolean paramsOk = fn.checkParameters(params);
                    for (int i = 0; i < count; i++) {
                        rng.drawn.clear();
                        String res;
                        if (!paramsOk) {
                            res = "E:FunctionParametersNotAcceptedException";
                        } else {
                            try {
                                res = result(fn.evaluate(params));
                            } catch (Throwable t) {
                                res = error(t);
                            }
                        }
                        StringBuilder sb = new StringBuilder();
                        for (double u : rng.drawn) {
                            if (sb.length() > 0) {
                                sb.append(',');
                            }
                            sb.append(hex(u));
                        }
                        w.println((sb.length() == 0 ? "-" : sb.toString()) + " " + res);
                        if (!paramsOk && i >= 2) {
                            break;
                        }
                    }
                }
                id++;
            }
        }
    }

    // ------------------------------------------------------------------------------------------

    @SuppressWarnings({ "unchecked", "rawtypes" })
    static void probfn(String out, int n, long seed) throws IOException {
        // Each case: PMF (probabilities, values are doubles) or boxed PDF (value;prob pairs), model order.
        Object[][] pmfs = {
                { new double[] { 1, 2, 3 }, new double[] { 0.2, 0.3, 0.5 } },
                { new double[] { 3, 1, 2 }, new double[] { 0.5, 0.2, 0.3 } }, // unsorted values
                { new double[] { 1, 2, 3, 4 }, new double[] { 0.1, 0.2, 0.3, 0.3 } }, // sum 0.9 -> adjusted
                { new double[] { 1, 2, 3 }, new double[] { 0.4, 0.0, 0.7 } }, // sum 1.1, zero sample
                { new double[] { 5 }, new double[] { 1.0 } }, // degenerate
                { new double[] { 1, 2 }, new double[] { 0.3333333333, 0.6666666666 } }, // tiny deficit
                { new double[] { 0.5, -1, 7, 2 }, new double[] { 0.1, 0.1, 0.1, 0.7 } },
                { new double[] { 1, 2, 3 }, new double[] { 0.1, 0.1, 0.1 } }, // sum 0.3 -> adjusted
                { new double[] { 1, 2 }, new double[] { 0.5, 0.49999 } }, // not adjusted? (>1e-9)
                { new double[] { 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12 },
                        new double[] { 0.05, 0.1, 0.05, 0.1, 0.05, 0.1, 0.05, 0.1, 0.05, 0.1, 0.05, 0.2 } },
                { new double[] { 1, 2 }, new double[] { 1.5, -0.5 } }, // invalid
        };
        Object[][] pdfs = {
                { new double[] { 1, 2 }, new double[] { 0.5, 0.5 } },
                { new double[] { 10, 2, 5 }, new double[] { 0.2, 0.3, 0.5 } },
                { new double[] { 1, 2, 3 }, new double[] { 0.2, 0.0, 0.8 } },
                { new double[] { 0.5, 1.5, 2.5, 3.5 }, new double[] { 0.1, 0.2, 0.3, 0.3 } }, // adjusted
                { new double[] { 1e-3, 2e-3, 1 }, new double[] { 0.25, 0.25, 0.5 } },
                { new double[] { 1, 2 }, new double[] { 0.3333333333, 0.6666666666 } },
                { new double[] { 0, 1 }, new double[] { 0.5, 0.5 } }, // value 0 -> Line error
                { new double[] { 1, 1 }, new double[] { 0.5, 0.5 } }, // duplicate
                { new double[] { 2 }, new double[] { 1.0 } },
        };
        ProbfunctionFactory pf = ProbfunctionFactory.eINSTANCE;
        try (PrintWriter w = open(out)) {
            w.println("# probfn: case pmf|pdf seed=<s> <values> / <probs>; per draw: <u> <result>");
            int id = 0;
            for (int kind = 0; kind < 2; kind++) {
                Object[][] list = kind == 0 ? pmfs : pdfs;
                for (Object[] cs : list) {
                    double[] vals = (double[]) cs[0];
                    double[] probs = (double[]) cs[1];
                    RecordingRng rng = new RecordingRng(seed + id, null);
                    ProbabilityFunctionFactoryImpl.getInstance().setRandomGenerator(rng);
                    ProbabilityFunctionLiteral lit = StoexFactory.eINSTANCE.createProbabilityFunctionLiteral();
                    Object model;
                    if (kind == 0) {
                        ProbabilityMassFunction pmf = pf.createProbabilityMassFunction();
                        for (int i = 0; i < vals.length; i++) {
                            Sample s = pf.createSample();
                            s.setValue(vals[i]);
                            s.setProbability(probs[i]);
                            pmf.getSamples().add(s);
                        }
                        model = pmf;
                        lit.setFunction_ProbabilityFunctionLiteral(pmf);
                    } else {
                        BoxedPDF pdf = pf.createBoxedPDF();
                        for (int i = 0; i < vals.length; i++) {
                            ContinuousSample s = pf.createContinuousSample();
                            s.setValue(vals[i]);
                            s.setProbability(probs[i]);
                            pdf.getSamples().add(s);
                        }
                        model = pdf;
                        lit.setFunction_ProbabilityFunctionLiteral(pdf);
                    }
                    StringBuilder hdr = new StringBuilder("case " + (kind == 0 ? "pmf" : "pdf") + " seed="
                            + (seed + id));
                    for (double v : vals) {
                        hdr.append(' ').append(hex(v));
                    }
                    hdr.append(" /");
                    for (double p : probs) {
                        hdr.append(' ').append(hex(p));
                    }
                    w.println(hdr);
                    ProbFunctionCache cache;
                    try {
                        cache = new ProbFunctionCache(lit);
                    } catch (Throwable t) {
                        w.println("- " + error(t));
                        id++;
                        continue;
                    }
                    Object fn = cache.getProbFunction((org.eclipse.emf.ecore.EObject) model);
                    for (int i = 0; i < n; i++) {
                        rng.drawn.clear();
                        String res;
                        try {
                            if (fn instanceof IProbabilityMassFunction) {
                                res = result(((IProbabilityMassFunction) fn).drawSample());
                            } else {
                                res = result(((IProbabilityDensityFunction) fn).drawSample());
                            }
                        } catch (Throwable t) {
                            res = error(t);
                        }
                        StringBuilder sb = new StringBuilder();
                        for (double u : rng.drawn) {
                            if (sb.length() > 0) {
                                sb.append(',');
                            }
                            sb.append(hex(u));
                        }
                        w.println((sb.length() == 0 ? "-" : sb.toString()) + " " + res);
                    }
                    id++;
                }
            }
        }
    }
}
