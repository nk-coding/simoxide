package cmpss;

import java.io.File;
import java.util.List;
import java.util.Map;

import org.eclipse.emf.common.util.URI;
import org.eclipse.emf.ecore.plugin.EcorePlugin;

import cmp.Harness;
import slingshot.headless.PlainMain;
import slingshot.headless.SlingshotRunner;

/**
 * Slingshot (nightly 2026-09-01) on a flat classpath, one run = palladio-research/work-slingshot
 * SlingshotRunner.run (blackboard load, in-memory EDP2, driver init + start, EDP2 read-back).
 * Slingshot ignores maximumMeasurementCount: only simTime stops it.
 *
 * Usage: java -cp classes:&lt;sl-install jars&gt; cmpss.CompareSlingshot model=DIR simTime=T seed=S
 * warmup=W runs=R [barrier=DIR tag=X]. {@link #instance} is the entry for isolated class loaders.
 */
public final class CompareSlingshot {

    public static void main(String[] args) throws Exception {
        Harness.silence();
        run(args);
        System.exit(0);
    }

    /** Entry point used by {@link IsoMain} inside an isolated class loader (no System.exit). */
    public static void instance(String[] args) throws Exception {
        run(args);
    }

    static void run(String[] args) throws Exception {
        Map<String, String> a = Harness.args(args, 0);
        EcorePlugin.ExtensionProcessor.process(null);
        PlainMain.bootstrapSlingshot();
        org.apache.log4j.Logger.getRootLogger().setLevel(org.apache.log4j.Level.toLevel(a.getOrDefault("log", "ERROR")));
        List<URI> models = SlingshotRunner.modelUris(new File(a.get("model")));
        double simTime = Double.parseDouble(a.getOrDefault("simTime", "1000"));
        Long seed = Long.valueOf(a.getOrDefault("seed", "1"));
        Harness.loop(a.getOrDefault("sim", "slingshot"), a, id -> {
            SlingshotRunner.Result r = SlingshotRunner.run(models, simTime, Long.MAX_VALUE, Harness.seed(seed, id), false);
            Harness.Res res = new Harness.Res();
            res.simMs = r.simMs;
            res.events = r.events;
            res.simEnd = simTime;
            for (String s : r.summaries) { // "Usage Scenario: X | Response Time Tuple: n=.. mean=.."
                if (s.startsWith("Usage Scenario") && s.contains("Response Time")) {
                    int n = s.lastIndexOf(" n=") + 1, m = s.lastIndexOf(" mean=");
                    res.requests = Long.parseLong(s.substring(n + 2, m));
                    res.meanRt = Double.parseDouble(s.substring(m + 6));
                }
            }
            if (res.requests <= 0) {
                res.error = "no usage-scenario response times (Slingshot swallows errors)";
            }
            return res;
        });
    }
}
