package refsim;

import java.io.File;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.OutputStreamWriter;
import java.io.StringWriter;
import java.io.Writer;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

/**
 * refsim: deterministic SimuLizar 5.2.2 reference runner.
 *
 * <pre>
 * refsim run --model &lt;dir | files,...&gt; [--seed N] [--max-sim-time T] [--max-measurements M]
 *            [--trace out.jsonl] [--tape out.jsonl] [--measurements out.csv] [--linking]
 * refsim run --run-json corpus/x/run.json [--trace ...] ...
 * refsim batch &lt;corpusDir&gt; [--out DIR | --in-place] [--only a,b] [--repeat N] [--check] [--no-trace]
 * </pre>
 */
public final class Main {

    static java.io.PrintStream OUT = System.out;
    static java.io.PrintStream ERR = System.err;

    private Main() {
    }

    public static void main(String[] args) throws Exception {
        if (args.length < 2) {
            usage();
        }
        // DESMO-J and OCL print to stdout/stderr; keep stdout for refsim's own results.
        OUT = System.out;
        java.io.PrintStream sink = Boolean.getBoolean("refsim.verbose") ? System.err
                : new java.io.PrintStream(java.io.OutputStream.nullOutputStream());
        System.setOut(sink);
        if (!Boolean.getBoolean("refsim.verbose")) {
            ERR = System.err;
            System.setErr(sink);
        }
        File plugins = new File(args[0]);
        String cmd = args[1];
        String[] rest = Arrays.copyOfRange(args, 2, args.length);
        long tb = System.nanoTime();
        Bootstrap.init(plugins);
        ERR.printf("[refsim] bootstrap %d ms (patched=%s)%n", (System.nanoTime() - tb) / 1_000_000,
                Statics.patched());
        int rc;
        try {
            rc = dispatch(cmd, rest);
        } catch (Throwable t) {
            ERR.println("[refsim] FAILED: " + t);
            t.printStackTrace(ERR);
            rc = 1;
        }
        System.exit(rc);
    }

    private static int dispatch(String cmd, String[] rest) throws Exception {
        int rc;
        switch (cmd) {
        case "run":
            rc = cmdRun(rest);
            break;
        case "batch":
            rc = cmdBatch(rest);
            break;
        case "import":
            rc = refsim.corpus.CorpusTool.importModel(rest);
            break;
        case "gen":
            rc = refsim.corpus.HandMade.generate(rest);
            break;
        default:
            usage();
            rc = 2;
        }
        return rc;
    }

    private static void usage() {
        ERR.println("usage: refsim run --model <dir|files> [--seed N] [--max-sim-time T] "
                + "[--max-measurements M] [--trace F] [--tape F] [--measurements F] [--linking]\n"
                + "       refsim run --run-json corpus/<m>/run.json [--trace F] [--tape F] [--measurements F]\n"
                + "       refsim batch <corpusDir> [--out DIR | --in-place] [--only a,b] [--repeat N] [--check]");
        System.exit(2);
    }

    private static Writer open(String path) throws IOException {
        if (path == null) {
            return null;
        }
        File f = new File(path).getAbsoluteFile();
        f.getParentFile().mkdirs();
        return new OutputStreamWriter(new FileOutputStream(f), StandardCharsets.UTF_8);
    }

    private static int cmdRun(String[] a) throws Exception {
        RunSpec spec = new RunSpec();
        String trace = null, tape = null, meas = null;
        for (int i = 0; i < a.length; i++) {
            switch (a[i]) {
            case "--model":
                spec.setModel(a[++i]);
                break;
            case "--run-json": {
                RunSpec r = RunSpec.fromRunJson(new File(a[++i]));
                spec.name = r.name;
                spec.usageModel = r.usageModel;
                spec.allocations = r.allocations;
                spec.monitorRepository = r.monitorRepository;
                spec.seed = r.seed;
                spec.maxSimTime = r.maxSimTime;
                spec.maxMeasurements = r.maxMeasurements;
                spec.simulateLinkingResources = r.simulateLinkingResources;
                spec.simulateThroughputOfLinkingResources = r.simulateThroughputOfLinkingResources;
                break;
            }
            case "--seed":
                spec.seed = Long.parseLong(a[++i]);
                break;
            case "--max-sim-time":
                spec.maxSimTime = Long.parseLong(a[++i]);
                break;
            case "--max-measurements":
                spec.maxMeasurements = Long.parseLong(a[++i]);
                break;
            case "--trace":
                trace = a[++i];
                break;
            case "--tape":
                tape = a[++i];
                break;
            case "--measurements":
                meas = a[++i];
                break;
            case "--linking":
                spec.simulateLinkingResources = true;
                break;
            case "--no-link-throughput":
                spec.simulateThroughputOfLinkingResources = false;
                break;
            default:
                throw new IllegalArgumentException("unknown option " + a[i]);
            }
        }
        Writer tw = open(trace), pw = open(tape);
        Runner.Result r;
        try {
            r = Runner.run(spec, tw, pw);
        } finally {
            if (tw != null) {
                tw.close();
            }
            if (pw != null) {
                pw.close();
            }
        }
        if (meas != null) {
            try (Writer mw = open(meas)) {
                r.measurements.writeCsv(mw);
            }
        }
        ERR.printf("[refsim] %s: %d ms, end t=%s, uniforms=%d, measurements=%d%n", spec.name,
                r.wallNanos / 1_000_000, Double.toString(r.endTime), r.uniforms, r.measurements.total);
        ERR.print(r.measurements.summary());
        if (r.error != null) {
            ERR.println("[refsim] RUN FAILED: " + r.error);
            r.error.printStackTrace(ERR);
            return 1;
        }
        return 0;
    }

    /** Output of one run held in memory (for batch/repeat/check). */
    static final class Out {
        String trace, tape, csv;
        Runner.Result r;

        String digest() throws Exception {
            MessageDigest md = MessageDigest.getInstance("SHA-256");
            md.update(trace.getBytes(StandardCharsets.UTF_8));
            md.update((byte) 0);
            md.update(tape.getBytes(StandardCharsets.UTF_8));
            md.update((byte) 0);
            md.update(csv.getBytes(StandardCharsets.UTF_8));
            StringBuilder sb = new StringBuilder();
            for (byte b : md.digest()) {
                sb.append(String.format("%02x", b));
            }
            return sb.substring(0, 16);
        }
    }

    static Out runToMemory(RunSpec spec, boolean withTrace) throws Exception {
        StringWriter tw = new StringWriter(1 << 20), pw = new StringWriter(1 << 16);
        Out o = new Out();
        o.r = Runner.run(spec, withTrace ? tw : null, withTrace ? pw : null);
        o.trace = tw.toString();
        o.tape = pw.toString();
        StringWriter mw = new StringWriter();
        o.r.measurements.writeCsv(mw);
        o.csv = mw.toString();
        return o;
    }

    private static int cmdBatch(String[] a) throws Exception {
        File corpus = new File(a[0]).getCanonicalFile();
        File out = null;
        boolean inPlace = false, check = false, withTrace = true;
        int repeat = 1;
        Set<String> only = null;
        for (int i = 1; i < a.length; i++) {
            switch (a[i]) {
            case "--out":
                out = new File(a[++i]).getAbsoluteFile();
                break;
            case "--in-place":
                inPlace = true;
                break;
            case "--only":
                only = new HashSet<>(Arrays.asList(a[++i].split(",")));
                break;
            case "--repeat":
                repeat = Integer.parseInt(a[++i]);
                break;
            case "--check":
                check = true;
                break;
            case "--no-trace":
                withTrace = false;
                break;
            default:
                throw new IllegalArgumentException("unknown option " + a[i]);
            }
        }
        List<File> models = new ArrayList<>();
        File[] ds = corpus.listFiles();
        Arrays.sort(ds);
        for (File d : ds) {
            if (new File(d, "run.json").isFile() && (only == null || only.contains(d.getName()))) {
                models.add(d);
            }
        }
        // warm-up: one untraced run of the first model (class loading, EMF/OCL lazy init)
        if (!models.isEmpty()) {
            long tw = System.nanoTime();
            RunSpec w = RunSpec.fromRunJson(new File(models.get(0), "run.json"));
            runToMemory(w, false);
            ERR.printf("[refsim] warm-up %d ms%n", (System.nanoTime() - tw) / 1_000_000);
        }
        int failures = 0;
        for (File d : models) {
            RunSpec spec = RunSpec.fromRunJson(new File(d, "run.json"));
            Out first = null;
            String status = "ok";
            for (int k = 0; k < repeat; k++) {
                Out o = runToMemory(spec, withTrace);
                if (o.r.error != null) {
                    status = "ERROR " + o.r.error;
                    o.r.error.printStackTrace(ERR);
                    failures++;
                    break;
                }
                if (first == null) {
                    first = o;
                } else if (!first.digest().equals(o.digest())) {
                    status = "NONDETERMINISTIC (repeat " + k + ": " + first.digest() + " vs " + o.digest() + ")";
                    failures++;
                    break;
                }
            }
            if (first != null && status.equals("ok")) {
                File exp = new File(d, "expected");
                if (check) {
                    String d1 = read(new File(exp, "trace.jsonl")), d2 = read(new File(exp, "tape.jsonl")),
                            d3 = read(new File(exp, "measurements.csv"));
                    if (!first.trace.equals(d1) || !first.tape.equals(d2) || !first.csv.equals(d3)) {
                        status = "MISMATCH vs expected/ (trace " + first.trace.equals(d1) + ", tape "
                                + first.tape.equals(d2) + ", csv " + first.csv.equals(d3) + ")";
                        failures++;
                    }
                }
                File target = inPlace ? exp : (out != null ? new File(out, d.getName()) : null);
                if (target != null) {
                    target.mkdirs();
                    Files.writeString(new File(target, "measurements.csv").toPath(), first.csv);
                    if (withTrace) {
                        Files.writeString(new File(target, "trace.jsonl").toPath(), first.trace);
                        Files.writeString(new File(target, "tape.jsonl").toPath(), first.tape);
                    }
                }
            }
            OUT.printf("%-40s %-8s %6d ms  t_end=%-14s uniforms=%-8d meas=%-7d trace=%-9d %s%n", d.getName(),
                    first == null ? "-" : first.digest().substring(0, 8), first == null ? 0 : first.r.wallNanos / 1_000_000,
                    first == null ? "-" : Double.toString(first.r.endTime), first == null ? 0 : first.r.uniforms,
                    first == null ? 0 : first.r.measurements.total,
                    first == null ? 0 : first.trace.length(), status);
        }
        OUT.println(failures == 0 ? "ALL OK" : failures + " FAILURE(S)");
        return failures == 0 ? 0 : 1;
    }

    /** Reads an expected file; transparently handles a .gz sibling. */
    static String read(File f) throws IOException {
        if (f.isFile()) {
            return Files.readString(f.toPath());
        }
        File gz = new File(f.getPath() + ".gz");
        if (gz.isFile()) {
            try (var in = new java.util.zip.GZIPInputStream(new java.io.FileInputStream(gz))) {
                return new String(in.readAllBytes(), StandardCharsets.UTF_8);
            }
        }
        return null;
    }
}
