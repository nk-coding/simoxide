package headless.simucom;

import java.io.File;
import java.lang.management.ManagementFactory;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

import javax.measure.Measure;
import javax.measure.unit.SI;

import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.emf.common.util.URI;
import org.eclipse.equinox.app.IApplication;
import org.eclipse.equinox.app.IApplicationContext;
import org.palladiosimulator.analyzer.workflow.core.ConstantsContainer;
import org.palladiosimulator.analyzer.workflow.core.configurations.AbstractCodeGenerationWorkflowRunConfiguration;
import org.palladiosimulator.edp2.impl.RepositoryManager;
import org.palladiosimulator.edp2.models.ExperimentData.DataSeries;
import org.palladiosimulator.edp2.models.ExperimentData.ExperimentGroup;
import org.palladiosimulator.edp2.models.ExperimentData.ExperimentRun;
import org.palladiosimulator.edp2.models.ExperimentData.ExperimentSetting;
import org.palladiosimulator.edp2.models.ExperimentData.Measurement;
import org.palladiosimulator.edp2.models.ExperimentData.MeasurementRange;
import org.palladiosimulator.edp2.models.Repository.LocalMemoryRepository;
import org.palladiosimulator.edp2.models.Repository.RepositoryFactory;
import org.palladiosimulator.edp2.util.MeasurementsUtility;
import org.palladiosimulator.metricspec.constants.MetricDescriptionConstants;

import de.uka.ipd.sdq.codegen.simucontroller.core.runconfig.SimuComWorkflowConfiguration;
import de.uka.ipd.sdq.simucomframework.core.SimuComConfig;
import de.uka.ipd.sdq.workflow.mdsd.blackboard.MDSDBlackboard;

/**
 * Headless SimuCom (Palladio 5.2.2) runner: load -> validate -> codegen -> (fix sources) -> JDT/PDE compile -> jar
 * -> install in OSGi dock -> simulate, N times sequentially in one JVM.
 *
 * Args (key=value): model=dir simTime=S runs=N seed=K [maxMeasurements=-1] [dump=true] [keep=true]
 */
public class SimuComApp implements IApplication {

    private static boolean DUMP;

    @Override
    public Object start(IApplicationContext context) throws Exception {
        long jvmStart = ManagementFactory.getRuntimeMXBean().getStartTime();
        System.out.println("[simucom] JVM start -> application start: " + (System.currentTimeMillis() - jvmStart) + " ms");
        String[] args = (String[]) context.getArguments().get("application.args");
        Map<String, String> a = new HashMap<>();
        for (String s : args) {
            int i = s.indexOf('=');
            if (i > 0) a.put(s.substring(0, i), s.substring(i + 1));
        }
        String model = a.get("model");
        String simTime = a.getOrDefault("simTime", "1000");
        String maxMeas = a.getOrDefault("maxMeasurements", "-1");
        int runs = Integer.parseInt(a.getOrDefault("runs", "1"));
        long seed = Long.parseLong(a.getOrDefault("seed", "1"));
        DUMP = Boolean.parseBoolean(a.getOrDefault("dump", "false"));
        boolean keep = Boolean.parseBoolean(a.getOrDefault("keep", "false"));

        LocalMemoryRepository repo = RepositoryFactory.eINSTANCE.createLocalMemoryRepository();
        RepositoryManager.addRepository(RepositoryManager.getCentralRepository(), repo);

        int failures = 0;
        int warmup = Integer.parseInt(a.getOrDefault("warmup", "0"));
        String barrier = a.get("barrier");
        long tMeasured = 0;
        double duration = Double.parseDouble(a.getOrDefault("duration", "0")); // sustained: run until S seconds passed
        boolean varySeed = Boolean.parseBoolean(a.getOrDefault("varySeed", "false"));
        if (duration > 0) runs = Integer.MAX_VALUE - warmup - 1;
        int measured = 0;
        for (int r = 1; r <= warmup + runs; r++) {
            if (r == warmup + 1) {
                if (barrier != null) { // bench/compare: start N processes at the same time
                    new File(barrier, "ready." + ProcessHandle.current().pid()).createNewFile();
                    while (!new File(barrier, "go").exists()) Thread.sleep(5);
                }
                tMeasured = System.nanoTime();
            }
            if (duration > 0 && r > warmup && System.nanoTime() - tMeasured > duration * 1e9) break;
            PHASE = r <= warmup ? "warm" : "run";
            RUN = r <= warmup ? r - 1 : r - warmup - 1;
            if (r > warmup) measured++;
            try {
                String line = runOnce(r, new File(model).getAbsoluteFile(), simTime, maxMeas, varySeed ? seed + r - 1 : seed,
                        keep, repo);
                System.out.println(line + String.format(java.util.Locale.ROOT, " t_end_ms=%.1f",
                        (System.nanoTime() - tMeasured) / 1e6));
            } catch (Throwable e) {
                failures++;
                System.out.println("[simucom] RUN FAILED: " + e);
                System.out.println(String.format(java.util.Locale.ROOT,
                        "RESULT sim=simucom model=%s phase=%s run=%d wall_ms=-1 sim_ms=-1 requests=-1 mean_rt=NaN sim_end=-1 events=-1 t_end_ms=%.1f error=%s",
                        new File(model).getName(), PHASE, RUN, (System.nanoTime() - tMeasured) / 1e6,
                        String.valueOf(e).replace(' ', '_')));
                for (Throwable c = e; c != null; c = c.getCause())
                    if (c instanceof org.eclipse.core.runtime.CoreException)
                        dumpStatus(((org.eclipse.core.runtime.CoreException) c).getStatus(), "  ");
                e.printStackTrace(System.out);
            }
            System.out.flush();
        }
        double wall = (System.nanoTime() - tMeasured) / 1e6;
        System.out.println(String.format(java.util.Locale.ROOT,
                "PAR sim=simucom model=%s threads=1 runs=%d wall_ms=%.1f runs_per_s=%.3f end_epoch_ms=%d",
                new File(model).getName(), measured, wall, measured / (wall / 1000.0), System.currentTimeMillis()));
        System.out.println("[simucom] failures=" + failures + " JVM start -> end: "
                + (System.currentTimeMillis() - jvmStart) + " ms");
        return IApplication.EXIT_OK;
    }

    private static String PHASE = "run";
    private static int RUN;

    private static String runOnce(int id, File d, String simTime, String maxMeas, long seed, boolean keep,
            LocalMemoryRepository repo) throws Exception {
        long t0 = System.nanoTime();
        PatchedSimuComJob.MARKS.clear();
        PatchedSimuComJob.simEnd = -1;
        Map<String, Object> p = new HashMap<>();
        p.put(SimuComConfig.SIMULATE_LINKING_RESOURCES, false);
        p.put(SimuComConfig.SIMULATE_THROUGHPUT_OF_LINKING_RESOURCES, true);
        p.put(SimuComConfig.SIMULATE_FAILURES, false);
        p.put(SimuComConfig.USE_FIXED_SEED, true);
        for (int i = 0; i < 6; i++)
            p.put(SimuComConfig.FIXED_SEED_PREFIX + i, Long.toString(seed + i)); // same seed words as refsim/SimuLizar
        p.put(SimuComConfig.PERSISTENCE_RECORDER_NAME, org.palladiosimulator.recorderframework.edp2.Activator.EDP2_ID);
        p.put(SimuComConfig.SIMULATOR_ID, "de.uka.ipd.sdq.codegen.simucontroller.simucom");
        p.put(SimuComConfig.EXPERIMENT_RUN, "simucom-" + id);
        p.put(SimuComConfig.SIMULATION_TIME, simTime);
        p.put(SimuComConfig.MAXIMUM_MEASUREMENT_COUNT, maxMeas);
        p.put(SimuComConfig.VARIATION_ID, "run-" + id);
        p.put(SimuComConfig.VERBOSE_LOGGING, false);
        p.put("EDP2RepositoryID", repo.getId());
        SimuComConfig scc = new SimuComConfig(p, false);
        SimuComWorkflowConfiguration cfg = new SimuComWorkflowConfiguration(p);
        cfg.setSimuComConfiguration(scc);
        cfg.setUsageModelFile(URI.createFileURI(find(d, ".usagemodel")).toString());
        List<String> allocs = new ArrayList<>();
        allocs.add(URI.createFileURI(find(d, ".allocation")).toString());
        cfg.setAllocationFiles(allocs);
        cfg.setRMIMiddlewareFile("pathmap://PCM_MODELS/Glassfish.repository");
        cfg.setEventMiddlewareFile("pathmap://PCM_MODELS/default_event_middleware.repository");
        cfg.setDebug(false);
        cfg.setInteractive(false);
        cfg.setCodeGenerationAdvicesFile(
                AbstractCodeGenerationWorkflowRunConfiguration.CodeGenerationAdvice.SIMULATION);
        String pid = ConstantsContainer.DEFAULT_TEMPORARY_DATA_LOCATION + ".r" + id;
        cfg.getAttributes().put(ConstantsContainer.TEMPORARY_DATA_LOCATION, pid);
        cfg.setStoragePluginID(pid);
        cfg.setOverwriteWithoutAsking(true);
        cfg.setDeleteTemporaryDataAfterAnalysis(!keep);
        cfg.setAccuracyInfluenceAnalysisEnabled(false);
        cfg.setSensitivityAnalysisEnabled(false);
        cfg.setFeatureConfigFile("pathmap://PCM_MODELS/ConnectorConfig.featureconfig");

        PatchedSimuComJob job = new PatchedSimuComJob(cfg);
        job.setBlackboard(new MDSDBlackboard());
        NullProgressMonitor pm = new NullProgressMonitor();
        long tStart = System.nanoTime();
        try {
            job.execute(pm);
        } finally {
            try {
                job.cleanup(pm);
            } catch (Exception e) {
                System.out.println("[simucom] cleanup failed: " + e);
            }
            try {
                scc.disposeRandomGenerator();
            } catch (Exception e) {
                // ignore
            }
        }
        long tJob = System.nanoTime();
        double[] rt = extract(repo);
        repo.getExperimentGroups().clear();
        long t3 = System.nanoTime();

        Map<String, Long> m = PatchedSimuComJob.MARKS;
        long simMs = m.containsKey("simulate") ? (m.get("simulate") - m.get("install")) / 1_000_000 : -1;
        StringBuilder ph = new StringBuilder("PHASES sim=simucom model=" + d.getName() + " run=" + id);
        long prev = tStart;
        ph.append(" setup_ms=").append((tStart - t0) / 1_000_000);
        String[][] names = { { "prepared", "load_validate_ms" }, { "codegen", "codegen_ms" },
                { "compile", "compile_ms" }, { "jar", "jar_ms" }, { "install", "install_ms" },
                { "simulate", "sim_ms" } };
        for (String[] n : names) {
            Long v = m.get(n[0]);
            if (v == null) break;
            ph.append(' ').append(n[1]).append('=').append((v - prev) / 1_000_000);
            prev = v;
        }
        ph.append(" cleanup_extract_ms=").append((t3 - prev) / 1_000_000);
        System.out.println(ph);
        return String.format(java.util.Locale.ROOT,
                "RESULT sim=simucom model=%s phase=%s run=%d wall_ms=%d sim_ms=%d requests=%d mean_rt=%s sim_end=%s events=-1",
                d.getName(), PHASE, RUN, (t3 - t0) / 1_000_000, simMs, (long) rt[1],
                rt[1] == 0 ? "NaN" : Double.toString(rt[0] / rt[1]),
                PatchedSimuComJob.simEnd < 0 ? "-1" : Double.toString(PatchedSimuComJob.simEnd));
    }

    /** sum and count of usage-scenario response times. */
    private static double[] extract(LocalMemoryRepository repo) {
        double sum = 0;
        long n = 0;
        for (ExperimentGroup g : repo.getExperimentGroups())
            for (ExperimentSetting s : g.getExperimentSettings())
                for (ExperimentRun run : s.getExperimentRuns())
                    for (Measurement m : run.getMeasurement()) {
                        var mp = m.getMeasuringType().getMeasuringPoint();
                        String mpType = mp.eClass().getName();
                        boolean isRt = m.getMeasuringType().getMetric().getId()
                                .equals(MetricDescriptionConstants.RESPONSE_TIME_METRIC_TUPLE.getId());
                        long k = 0;
                        double ks = 0;
                        if (isRt)
                            for (MeasurementRange mr : m.getMeasurementRanges()) {
                                DataSeries ds = mr.getRawMeasurements().getDataSeries().get(1);
                                for (Object o : MeasurementsUtility.getMeasurementsDao(ds).getMeasurements()) {
                                    ks += ((Measure<?, ?>) o).doubleValue((javax.measure.unit.Unit) SI.SECOND);
                                    k++;
                                }
                            }
                        if (DUMP)
                            System.out.println("[simucom]   measurement: " + m.getMeasuringType().getMetric().getName()
                                    + " @ " + mpType + " " + mp.getStringRepresentation()
                                    + (isRt ? " n=" + k + " mean=" + (ks / k) : ""));
                        if (isRt && (mpType.startsWith("UsageScenario")
                                || String.valueOf(mp.getStringRepresentation()).startsWith("Usage Scenario"))) {
                            sum += ks;
                            n += k;
                        }
                    }
        return new double[] { sum, n };
    }

    private static void dumpStatus(org.eclipse.core.runtime.IStatus st, String ind) {
        System.out.println("[simucom] status" + ind + st.getMessage()
                + (st.getException() != null ? " / " + st.getException() : ""));
        for (var c : st.getChildren()) dumpStatus(c, ind + "  ");
    }

    private static String find(File dir, String ext) {
        File[] fs = dir.listFiles();
        if (fs == null) return null;
        java.util.Arrays.sort(fs);
        for (File f : fs)
            if (f.isFile() && f.getName().endsWith(ext)) return f.getAbsolutePath();
        return null;
    }

    @Override
    public void stop() {
    }
}
