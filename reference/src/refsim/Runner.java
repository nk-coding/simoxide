package refsim;

import java.io.File;
import java.io.IOException;
import java.io.Writer;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

import org.eclipse.core.runtime.NullProgressMonitor;
import org.eclipse.emf.common.util.URI;
import org.palladiosimulator.simulizar.SimuLizarPlatform;
import org.palladiosimulator.simulizar.core.runconfig.SimuLizarWorkflowConfiguration;
import org.palladiosimulator.simulizar.di.component.core.SimuLizarRuntimeComponent;
import org.palladiosimulator.simulizar.di.component.dependency.SimEngineComponent;
import org.palladiosimulator.simulizar.di.modules.stateless.core.RootComponentFactoriesModule;

import de.uka.ipd.sdq.simucomframework.core.SimuComConfig;
import de.uka.ipd.sdq.simulation.abstractsimengine.ISimEngineFactory;
import de.uka.ipd.sdq.simulation.abstractsimengine.ISimEventFactory;
import de.uka.ipd.sdq.simulation.abstractsimengine.desmoj.DesmoJSimEngineFactory;
import de.uka.ipd.sdq.workflow.jobs.IJob;
import refsim.trace.Trace;

/** Executes one SimuLizar 5.2.2 simulation run in this JVM. */
public final class Runner {

    public static final class Result {
        public Measurements measurements;
        public long wallNanos;
        public long uniforms;
        public double endTime;
        public Throwable error;
    }

    private static int runCounter;

    private Runner() {
    }

    public static Result run(RunSpec spec, Writer traceOut, Writer tapeOut) throws Exception {
        spec.validate();
        Result res = new Result();
        long t0 = System.nanoTime();
        int id = ++runCounter;
        Statics.resetForNewRun();

        Map<String, Object> p = new HashMap<>();
        p.put(SimuComConfig.SIMULATE_LINKING_RESOURCES, spec.simulateLinkingResources);
        p.put(SimuComConfig.SIMULATE_THROUGHPUT_OF_LINKING_RESOURCES, spec.simulateThroughputOfLinkingResources);
        p.put(SimuComConfig.SIMULATE_FAILURES, false);
        p.put(SimuComConfig.USE_FIXED_SEED, true);
        long[] w = spec.seedWords();
        for (int i = 0; i < 6; i++) {
            p.put(SimuComConfig.FIXED_SEED_PREFIX + i, Long.toString(w[i]));
        }
        p.put(SimuComConfig.PERSISTENCE_RECORDER_NAME, Bootstrap.RECORDER_NAME);
        p.put(SimuComConfig.SIMULATOR_ID, "de.uka.ipd.sdq.codegen.simucontroller.simulizar");
        p.put(SimuComConfig.EXPERIMENT_RUN, "refsim");
        p.put(SimuComConfig.SIMULATION_TIME, Long.toString(spec.maxSimTime));
        p.put(SimuComConfig.MAXIMUM_MEASUREMENT_COUNT, Long.toString(spec.maxMeasurements));
        p.put(SimuComConfig.VARIATION_ID, "refsim");
        p.put(SimuComConfig.VERBOSE_LOGGING, false);
        SimuLizarWorkflowConfiguration cfg = new SimuLizarWorkflowConfiguration(p);
        SimuComConfig scc = new SimuComConfig(p, false);
        cfg.setSimuComConfiguration(scc);
        cfg.setUsageModelFile(URI.createFileURI(spec.usageModel.getCanonicalPath()).toString());
        List<String> allocs = new ArrayList<>();
        for (File a : spec.allocations) {
            allocs.add(URI.createFileURI(a.getCanonicalPath()).toString());
        }
        cfg.setAllocationFiles(allocs);
        if (spec.monitorRepository != null) {
            cfg.setMonitorRepositoryFile(URI.createFileURI(spec.monitorRepository.getCanonicalPath()).toString());
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
        // The Eclipse SimEngineComponent reads the engine from Eclipse preferences; its default is the
        // first registered engine extension, which in the 5.2.2 product is DESMO-J (the only one).
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

        Measurements.begin();
        Trace.begin(traceOut, tapeOut, () -> 0.0);
        if (Trace.ON) {
            Trace.event("header", null, "format", "palladio-trace/1", "run", spec.name, "seed", spec.seed,
                    "max_sim_time", spec.maxSimTime, "max_measurements", spec.maxMeasurements);
        }
        var pm = new NullProgressMonitor();
        try {
            try {
                job.execute(pm);
            } finally {
                job.cleanup(pm);
            }
        } catch (Throwable e) {
            res.error = e;
        } finally {
            res.endTime = Statics.currentTime();
            if (Trace.ON) {
                Trace.event("finish", null, "uniforms", Trace.uniformCount(), "measurements",
                        Measurements.current().total);
            }
            res.uniforms = Trace.uniformCount();
            try {
                Trace.end();
            } catch (IOException e) {
                if (res.error == null) {
                    res.error = e;
                }
            }
            res.measurements = Measurements.end();
            try {
                scc.disposeRandomGenerator();
            } catch (Exception e) {
                // ignore
            }
            Statics.afterRun();
        }
        res.wallNanos = System.nanoTime() - t0;
        return res;
    }
}
