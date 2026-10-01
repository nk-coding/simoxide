package cmpsl;

import java.io.File;
import java.io.InputStream;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.jar.JarFile;
import java.util.jar.Manifest;
import java.util.zip.ZipEntry;

import javax.measure.Measure;
import javax.measure.unit.SI;

import org.eclipse.core.runtime.ContributorFactorySimple;
import org.eclipse.core.runtime.IExtensionRegistry;
import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.core.runtime.RegistryFactory;
import org.eclipse.emf.common.util.URI;
import org.eclipse.emf.ecore.plugin.EcorePlugin;
import org.eclipse.emf.ecore.resource.URIConverter;
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
import org.palladiosimulator.simulizar.SimuLizarPlatform;
import org.palladiosimulator.simulizar.core.runconfig.SimuLizarWorkflowConfiguration;
import org.palladiosimulator.simulizar.di.component.core.SimuLizarRuntimeComponent;
import org.palladiosimulator.simulizar.di.component.dependency.SimEngineComponent;
import org.palladiosimulator.simulizar.di.modules.stateless.core.RootComponentFactoriesModule;

import cmp.Harness;
import de.uka.ipd.sdq.simucomframework.core.SimuComConfig;
import de.uka.ipd.sdq.simulation.abstractsimengine.ISimEngineFactory;
import de.uka.ipd.sdq.simulation.abstractsimengine.ISimEventFactory;
import de.uka.ipd.sdq.simulation.abstractsimengine.desmoj.DesmoJSimEngineFactory;
import de.uka.ipd.sdq.workflow.jobs.IJob;

/**
 * UNPATCHED SimuLizar 5.2.2 (stock product classes) without OSGi, recording into an in-memory EDP2
 * repository as a user of the product would. Bootstrap from
 * palladio-research/work-simucom/standalone/StandaloneSimuLizar (extension registry from every
 * plugin.xml, EMF ExtensionProcessor, platform:/plugin mappings, DESMO-J engine component).
 *
 * Usage: java -cp classes:&lt;OSGi-ordered product classpath&gt; cmpsl.CompareSimuLizar &lt;pluginsDir&gt;
 * model=DIR simTime=T maxMeas=M seed=S warmup=W runs=R threads=T [disposeRng=true|false] [sim=name]
 */
public final class CompareSimuLizar {

    private static final Object REPO_LOCK = new Object();
    private static boolean disposeRng;
    private static final ThreadLocal<LocalMemoryRepository> THREAD_REPO = ThreadLocal.withInitial(() -> {
        LocalMemoryRepository r = RepositoryFactory.eINSTANCE.createLocalMemoryRepository();
        synchronized (REPO_LOCK) {
            RepositoryManager.addRepository(RepositoryManager.getCentralRepository(), r);
        }
        return r;
    });

    public static void main(String[] args) throws Exception {
        Harness.silence();
        Map<String, String> a = Harness.args(args, 1);
        bootstrap(new File(args[0]));
        org.apache.log4j.BasicConfigurator.configure();
        org.apache.log4j.Logger.getRootLogger().setLevel(org.apache.log4j.Level.toLevel(a.getOrDefault("log", "ERROR")));
        int threads = Integer.parseInt(a.getOrDefault("threads", "1"));
        // SimuLizar never disposes its RNG (1 leaked producer thread per run). Disposing is only safe
        // sequentially: the RNG also sits in a static singleton shared by concurrent runs.
        disposeRng = Boolean.parseBoolean(a.getOrDefault("disposeRng", String.valueOf(threads <= 1)));
        String dir = a.get("model");
        String simTime = a.getOrDefault("simTime", "-1");
        String maxMeas = a.getOrDefault("maxMeas", "-1");
        long seed = Long.parseLong(a.getOrDefault("seed", "1"));
        Harness.loop(a.getOrDefault("sim", "simulizar"), a, id -> runOnce(id, new File(dir), simTime, maxMeas, Harness.seed(seed, id)));
        System.exit(0);
    }

    static void bootstrap(File pluginsDir) throws Exception {
        String exclude = "^org\\.palladiosimulator\\.(architecturaltemplates\\.(jobs|ui)|simulizar\\.action.*)$";
        Object masterToken = new Object();
        IExtensionRegistry registry = RegistryFactory.createRegistry(null, masterToken, null);
        File[] jars = pluginsDir.listFiles((d, n) -> n.endsWith(".jar"));
        java.util.Arrays.sort(jars);
        for (File jar : jars) {
            try (JarFile jf = new JarFile(jar)) {
                Manifest mf = jf.getManifest();
                if (mf == null) {
                    continue;
                }
                String bsn = mf.getMainAttributes().getValue("Bundle-SymbolicName");
                if (bsn == null) {
                    continue;
                }
                bsn = bsn.split(";")[0].trim();
                URIConverter.URI_MAP.put(URI.createURI("platform:/plugin/" + bsn + "/"),
                        URI.createURI("jar:" + jar.toURI() + "!/"));
                if (bsn.matches(exclude)) {
                    continue;
                }
                ZipEntry pe = jf.getEntry("plugin.xml");
                if (pe == null) {
                    continue;
                }
                try (InputStream in = jf.getInputStream(pe)) {
                    registry.addContribution(in, ContributorFactorySimple.createContributor(bsn), false, bsn, null,
                            masterToken);
                }
            }
        }
        RegistryFactory.setDefaultRegistryProvider(() -> registry);
        ClassLoader noPluginXml = new ClassLoader(CompareSimuLizar.class.getClassLoader()) {
            @Override
            public java.util.Enumeration<java.net.URL> getResources(String name) throws java.io.IOException {
                return "plugin.xml".equals(name) ? java.util.Collections.emptyEnumeration() : super.getResources(name);
            }
        };
        EcorePlugin.ExtensionProcessor.process(noPluginXml);
    }

    static Harness.Res runOnce(int id, File d, String simTime, String maxMeas, long seed) throws Exception {
        d = d.getAbsoluteFile();
        LocalMemoryRepository repo = THREAD_REPO.get();
        Map<String, Object> p = new java.util.HashMap<>();
        p.put(SimuComConfig.SIMULATE_LINKING_RESOURCES, false);
        p.put(SimuComConfig.SIMULATE_THROUGHPUT_OF_LINKING_RESOURCES, true);
        p.put(SimuComConfig.SIMULATE_FAILURES, false);
        p.put(SimuComConfig.USE_FIXED_SEED, true);
        for (int i = 0; i < 6; i++) {
            p.put(SimuComConfig.FIXED_SEED_PREFIX + i, Long.toString(seed + i)); // same seed words as refsim
        }
        p.put(SimuComConfig.PERSISTENCE_RECORDER_NAME, org.palladiosimulator.recorderframework.edp2.Activator.EDP2_ID);
        p.put(SimuComConfig.SIMULATOR_ID, "de.uka.ipd.sdq.codegen.simucontroller.simulizar");
        p.put(SimuComConfig.EXPERIMENT_RUN, "cmp-" + id);
        p.put(SimuComConfig.SIMULATION_TIME, simTime);
        p.put(SimuComConfig.MAXIMUM_MEASUREMENT_COUNT, maxMeas);
        p.put(SimuComConfig.VARIATION_ID, "run-" + id);
        p.put(SimuComConfig.VERBOSE_LOGGING, false);
        p.put("EDP2RepositoryID", repo.getId());
        SimuLizarWorkflowConfiguration cfg = new SimuLizarWorkflowConfiguration(p);
        final SimuComConfig scc;
        synchronized (REPO_LOCK) { // iterates the (non-thread-safe) central EDP2 repository list
            scc = new SimuComConfig(p, false);
        }
        cfg.setSimuComConfiguration(scc);
        cfg.setUsageModelFile(URI.createFileURI(find(d, ".usagemodel")).toString());
        List<String> allocs = new ArrayList<>();
        allocs.add(URI.createFileURI(find(d, ".allocation")).toString());
        cfg.setAllocationFiles(allocs);
        String mon = find(d, ".monitorrepository");
        if (mon != null) {
            cfg.setMonitorRepositoryFile(URI.createFileURI(mon).toString());
        }
        cfg.setReconfigurationRulesFolder("");
        cfg.setServiceLevelObjectivesFile("");
        cfg.setUsageEvolutionFile("");
        cfg.setRMIMiddlewareFile("pathmap://PCM_MODELS/Glassfish.repository");
        cfg.setEventMiddlewareFile("pathmap://PCM_MODELS/default_event_middleware.repository");
        cfg.setDebug(false);
        cfg.setInteractive(false);
        cfg.setOverwriteWithoutAsking(true);

        var factory = SimuLizarPlatform.getPlatformComponent().analysisFactory();
        RootComponentFactoriesModule base = factory.defaultComponentFactoriesModule();
        RootComponentFactoriesModule mod = new RootComponentFactoriesModule() {
            @Override
            public SimuLizarRuntimeComponent.Factory providesRuntimeComponentFactory() {
                return base.providesRuntimeComponentFactory();
            }

            @Override
            public SimEngineComponent.Factory providesSimEngineComponentFactory() {
                return () -> {
                    final DesmoJSimEngineFactory f = new DesmoJSimEngineFactory();
                    return new SimEngineComponent() {
                        @Override
                        public ISimEngineFactory simEngineFactory() {
                            return f;
                        }

                        @Override
                        public ISimEventFactory simEventFactory() {
                            return f;
                        }
                    };
                };
            }
        };
        var root = factory.create(cfg, mod, factory.defaultExtensionComponentsModule(),
                factory.defaultMDSDBlackboardProvidingModule());
        IJob job = root.rootJob();
        var pm = new NullProgressMonitor();
        Harness.Res res = new Harness.Res();
        long t1 = System.nanoTime();
        try {
            job.execute(pm);
        } finally {
            try {
                job.cleanup(pm);
            } catch (Exception e) {
                // ignore
            }
            if (disposeRng) {
                try {
                    scc.disposeRandomGenerator();
                } catch (Exception e) {
                    // ignore
                }
            }
        }
        res.simMs = (System.nanoTime() - t1) / 1e6;
        // read the usage-scenario response times back from EDP2 (what a user's analysis would do)
        for (ExperimentGroup g : repo.getExperimentGroups()) {
            for (ExperimentSetting s : g.getExperimentSettings()) {
                for (ExperimentRun run : s.getExperimentRuns()) {
                    for (Measurement m : run.getMeasurement()) {
                        if (!m.getMeasuringType().getMetric().getId()
                                .equals(MetricDescriptionConstants.RESPONSE_TIME_METRIC_TUPLE.getId())) {
                            continue;
                        }
                        if (!m.getMeasuringType().getMeasuringPoint().eClass().getName()
                                .equals("UsageScenarioMeasuringPoint")) {
                            continue;
                        }
                        long k = 0;
                        double sum = 0;
                        for (MeasurementRange mr : m.getMeasurementRanges()) {
                            DataSeries ds = mr.getRawMeasurements().getDataSeries().get(1);
                            for (Object o : MeasurementsUtility.getMeasurementsDao(ds).getMeasurements()) {
                                @SuppressWarnings({ "rawtypes", "unchecked" })
                                double v = ((Measure) o).doubleValue((javax.measure.unit.Unit) SI.SECOND);
                                sum += v;
                                k++;
                            }
                        }
                        res.requests = k;
                        res.meanRt = k == 0 ? Double.NaN : sum / k;
                    }
                }
            }
        }
        repo.getExperimentGroups().clear();
        return res;
    }

    static String find(File dir, String ext) {
        File[] fs = dir.listFiles();
        if (fs == null) {
            return null;
        }
        java.util.Arrays.sort(fs);
        for (File f : fs) {
            if (f.isFile() && f.getName().endsWith(ext)) {
                return f.getAbsolutePath();
            }
        }
        return null;
    }
}
