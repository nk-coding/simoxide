package cmp;

import java.io.File;
import java.io.OutputStream;
import java.io.PrintStream;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;

/**
 * Shared benchmark loop of the JVM drivers (bench/compare). All drivers take the same key=value
 * arguments and print the same machine-readable lines:
 *
 * <pre>
 * RESULT sim=S model=M phase=warm|run run=I wall_ms=.. sim_ms=.. requests=.. mean_rt=.. sim_end=.. events=..
 * PAR sim=S model=M threads=T runs=R wall_ms=.. runs_per_s=.. end_epoch_ms=..
 * </pre>
 *
 * Arguments: model=DIR simTime=T maxMeas=M seed=S warmup=W runs=R threads=T [barrier=DIR] [tag=X]
 * With barrier=DIR the driver, after its warm-up, creates DIR/ready.&lt;tag&gt; and waits for DIR/go
 * (used to start N processes / isolated instances at the same time).
 */
public final class Harness {

    // IsoMain (isolated class loaders) hands the real stdout over through the system properties map
    public static PrintStream OUT = System.getProperties().get("cmp.realOut") instanceof PrintStream p ? p : System.out;
    public static PrintStream ERR = System.getProperties().get("cmp.realErr") instanceof PrintStream p ? p : System.err;
    private static boolean silenced;

    private Harness() {
    }

    public static Map<String, String> args(String[] a, int from) {
        Map<String, String> m = new HashMap<>();
        for (int i = from; i < a.length; i++) {
            int k = a[i].indexOf('=');
            if (k > 0) {
                m.put(a[i].substring(0, k), a[i].substring(k + 1));
            }
        }
        return m;
    }

    /** Routes System.out/err (DESMO-J, OCL, EMF chatter) to a null stream; results go to OUT. */
    public static synchronized void silence() {
        if (silenced || Boolean.getBoolean("cmp.verbose")) {
            return;
        }
        silenced = true;
        if (!(System.getProperties().get("cmp.realOut") instanceof PrintStream)) {
            OUT = System.out;
            ERR = System.err;
        }
        PrintStream sink = new PrintStream(OutputStream.nullOutputStream());
        System.setOut(sink);
        System.setErr(sink);
    }

    public static final class Res {
        public double wallMs;
        public double simMs = -1;
        public long requests = -1;
        public double meanRt = Double.NaN;
        public double simEnd = -1;
        public long events = -1;
        public String error;
        long tEndNanos;
    }

    /** Start of the measured phase (after warm-up and barrier); RESULT t_end_ms is relative to it. */
    static volatile long T0 = System.nanoTime();
    /** Epoch ms corresponding to T0. */
    static volatile long T0_EPOCH = System.currentTimeMillis();
    private static boolean varySeed;

    /** Seed of run `id` (1-based): base, or base + id - 1 with varySeed=true (sustained runs). */
    public static long seed(long base, int id) {
        return varySeed ? base + id - 1 : base;
    }

    /** Optional in-run progress probe, printed as "PROGRESS t_ms=.. &lt;probe&gt;" every progress=MS. */
    public static volatile java.util.function.Supplier<String> progress;

    public interface Run {
        Res run(int id) throws Exception;
    }

    static void print(String sim, String model, String phase, int i, Res r) {
        synchronized (Harness.class) {
            OUT.print(String.format(Locale.ROOT,
                    "RESULT sim=%s model=%s phase=%s run=%d wall_ms=%.3f sim_ms=%.3f requests=%d mean_rt=%.6f sim_end=%.3f events=%d t_end_ms=%.1f epoch_ms=%d%s%n",
                    sim, model, phase, i, r.wallMs, r.simMs, r.requests, r.meanRt, r.simEnd, r.events,
                    (r.tEndNanos - T0) / 1e6, T0_EPOCH + (r.tEndNanos - T0) / 1_000_000,
                    r.error == null ? "" : " error=" + r.error.replace(' ', '_')));
            OUT.flush();
        }
    }

    static Res timed(Run fn, int id) {
        long t0 = System.nanoTime();
        Res r;
        try {
            r = fn.run(id);
        } catch (Throwable t) {
            r = new Res();
            r.error = String.valueOf(t);
            t.printStackTrace(ERR);
        }
        r.tEndNanos = System.nanoTime();
        r.wallMs = (r.tEndNanos - t0) / 1e6;
        return r;
    }

    public static void barrier(String dir, String tag) throws InterruptedException {
        if (dir == null) {
            return;
        }
        try {
            new File(dir, "ready." + tag).createNewFile();
        } catch (java.io.IOException e) {
            throw new RuntimeException(e);
        }
        File go = new File(dir, "go");
        while (!go.exists()) {
            Thread.sleep(5);
        }
    }

    /**
     * Warm-up (sequential), optional barrier, then either `runs` measured runs on `threads` threads, or
     * with duration=S: `threads` workers each running back-to-back until S seconds have passed.
     */
    public static void loop(String sim, Map<String, String> a, Run fn) throws Exception {
        String model = new File(a.get("model")).getName();
        int warmup = Integer.parseInt(a.getOrDefault("warmup", "0"));
        int runs = Integer.parseInt(a.getOrDefault("runs", "1"));
        int threads = Integer.parseInt(a.getOrDefault("threads", "1"));
        double duration = Double.parseDouble(a.getOrDefault("duration", "0"));
        varySeed = Boolean.parseBoolean(a.getOrDefault("varySeed", "false"));
        String tag = a.getOrDefault("tag", String.valueOf(ProcessHandle.current().pid()));
        int progressMs = Integer.parseInt(a.getOrDefault("progress", "0"));
        if (progressMs > 0) {
            Thread p = new Thread(() -> {
                long start = System.nanoTime();
                while (true) {
                    try {
                        Thread.sleep(progressMs);
                    } catch (InterruptedException e) {
                        return;
                    }
                    java.util.function.Supplier<String> s = progress;
                    String v;
                    try {
                        v = s == null ? "" : s.get();
                    } catch (Throwable t) {
                        v = "probe_error=" + t.getClass().getSimpleName();
                    }
                    synchronized (Harness.class) {
                        OUT.print(String.format(Locale.ROOT, "PROGRESS t_ms=%d epoch_ms=%d %s%n",
                                (System.nanoTime() - start) / 1_000_000, System.currentTimeMillis(), v));
                        OUT.flush();
                    }
                }
            }, "cmp-progress");
            p.setDaemon(true);
            p.start();
        }
        java.util.concurrent.atomic.AtomicInteger ids = new java.util.concurrent.atomic.AtomicInteger();
        for (int w = 0; w < warmup; w++) {
            print(sim, model, "warm", w, timed(fn, ids.incrementAndGet()));
        }
        barrier(a.get("barrier"), tag);
        T0 = System.nanoTime();
        T0_EPOCH = System.currentTimeMillis();
        long t0 = T0;
        java.util.concurrent.atomic.AtomicInteger done = new java.util.concurrent.atomic.AtomicInteger();
        if (duration > 0) {
            long deadline = t0 + (long) (duration * 1e9);
            List<Thread> ws = new ArrayList<>();
            for (int k = 0; k < Math.max(1, threads); k++) {
                Thread t = new Thread(() -> {
                    while (System.nanoTime() < deadline) {
                        Res r = timed(fn, ids.incrementAndGet());
                        print(sim, model, "run", done.getAndIncrement(), r);
                    }
                }, "cmp-worker-" + k);
                ws.add(t);
                t.start();
            }
            for (Thread t : ws) {
                t.join();
            }
            runs = done.get();
        } else if (threads <= 1) {
            for (int i = 0; i < runs; i++) {
                print(sim, model, "run", i, timed(fn, ids.incrementAndGet()));
            }
        } else {
            ExecutorService pool = Executors.newFixedThreadPool(threads);
            List<Future<Res>> fs = new ArrayList<>();
            for (int i = 0; i < runs; i++) {
                final int rid = ids.incrementAndGet();
                fs.add(pool.submit(() -> timed(fn, rid)));
            }
            int i = 0;
            for (Future<Res> f : fs) {
                print(sim, model, "run", i++, f.get());
            }
            pool.shutdown();
        }
        double wall = (System.nanoTime() - t0) / 1e6;
        synchronized (Harness.class) {
            OUT.print(String.format(Locale.ROOT, "PAR sim=%s model=%s threads=%d runs=%d wall_ms=%.1f runs_per_s=%.3f end_epoch_ms=%d%n",
                    sim, model, threads, runs, wall, runs / (wall / 1000.0), System.currentTimeMillis()));
            OUT.flush();
        }
    }
}
