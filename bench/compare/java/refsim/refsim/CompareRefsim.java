package refsim;

import java.io.File;
import java.lang.reflect.Field;
import java.util.List;
import java.util.Map;

import cmp.Harness;

/**
 * Patched deterministic SimuLizar 5.2.2 (reference/refsim) in no-trace mode, driven by the shared
 * benchmark loop. Usage: java -cp ... refsim.CompareRefsim &lt;pluginsDir&gt; model=DIR simTime=T
 * maxMeas=M seed=S warmup=W runs=R
 */
public final class CompareRefsim {

    public static void main(String[] args) throws Exception {
        Harness.silence();
        Map<String, String> a = Harness.args(args, 1);
        Bootstrap.init(new File(args[0]));
        String dir = a.get("model");
        long simTime = Long.parseLong(a.getOrDefault("simTime", "-1"));
        long maxMeas = Long.parseLong(a.getOrDefault("maxMeas", "-1"));
        long seed = Long.parseLong(a.getOrDefault("seed", "1"));
        // in-run progress (progress=MS): simulated time and recorded tuples of the current run
        Harness.progress = () -> {
            long n;
            try {
                n = Measurements.current().total;
            } catch (IllegalStateException e) {
                n = -1;
            }
            return String.format(java.util.Locale.ROOT, "sim_time=%.1f measurements=%d heap_mb=%d", refsim.trace.Trace.now(), n,
                    (Runtime.getRuntime().totalMemory() - Runtime.getRuntime().freeMemory()) >> 20);
        };
        Harness.loop("refsim", a, id -> {
            RunSpec spec = new RunSpec();
            spec.setModel(dir);
            spec.seed = Harness.seed(seed, id);
            spec.maxSimTime = simTime;
            spec.maxMeasurements = maxMeas;
            long t0 = System.nanoTime();
            Runner.Result r = Runner.run(spec, null, null);
            Harness.Res res = new Harness.Res();
            res.simMs = (System.nanoTime() - t0) / 1e6; // Runner.run covers the whole job (load .. cleanup)
            res.simEnd = r.endTime;
            if (r.error != null) {
                res.error = String.valueOf(r.error);
            }
            scenario(r.measurements, res);
            return res;
        });
        System.exit(0);
    }

    @SuppressWarnings("unchecked")
    static void scenario(Measurements m, Harness.Res res) throws Exception {
        Field f = Measurements.class.getDeclaredField("series");
        f.setAccessible(true);
        for (Measurements.Series s : ((Map<String, Measurements.Series>) f.get(m)).values()) {
            if (s.mp.startsWith("UsageScenarioMeasuringPoint") && s.metric.startsWith("Response Time")) {
                List<double[]> rows = s.rows;
                double sum = 0;
                for (double[] row : rows) {
                    sum += row[1];
                }
                res.requests = rows.size();
                res.meanRt = rows.isEmpty() ? Double.NaN : sum / rows.size();
                return;
            }
        }
    }
}
