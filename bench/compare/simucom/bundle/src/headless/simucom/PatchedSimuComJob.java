package headless.simucom;

import java.util.LinkedHashMap;
import java.util.Map;

import org.eclipse.core.runtime.CoreException;
import org.eclipse.core.runtime.IProgressMonitor;

import de.uka.ipd.sdq.codegen.simucontroller.SimuControllerPlugin;
import de.uka.ipd.sdq.codegen.simucontroller.core.runconfig.SimuComWorkflowConfiguration;
import de.uka.ipd.sdq.codegen.simucontroller.core.dockmodel.DockModel;
import de.uka.ipd.sdq.codegen.simucontroller.workflow.jobs.AbstractSimulationJob;
import de.uka.ipd.sdq.codegen.simucontroller.workflow.jobs.BuildPluginJarJob;
import de.uka.ipd.sdq.codegen.simucontroller.workflow.jobs.CompilePluginCodeJob;
import de.uka.ipd.sdq.codegen.simucontroller.workflow.jobs.CreateSimuComMetaDataFilesJob;
import de.uka.ipd.sdq.codegen.simucontroller.workflow.jobs.DetermineFailureTypesJob;
import de.uka.ipd.sdq.codegen.simucontroller.workflow.jobs.XtendTransformPCMToCodeJob;
import de.uka.ipd.sdq.simucomframework.core.model.SimuComModel;
import de.uka.ipd.sdq.simucomframework.simulationdock.SimulationDockService;
import de.uka.ipd.sdq.simucomframework.simulationdock.SimulationDockServiceImpl;
import de.uka.ipd.sdq.workflow.jobs.CleanupFailedException;
import de.uka.ipd.sdq.workflow.jobs.IJob;
import de.uka.ipd.sdq.workflow.jobs.JobFailedException;
import de.uka.ipd.sdq.workflow.jobs.UserCanceledException;

/**
 * Same job sequence as SimuComJob (5.2.2), minus the workflow extension hooks, plus: a source-fix job between code
 * generation and compilation, timing marks between the phases, and a dock job that separates bundle
 * install/prepare from the simulation and keeps a handle on the SimuComModel (for the simulated end time).
 */
public class PatchedSimuComJob extends AbstractSimulationJob<SimuComWorkflowConfiguration> {

    /** phase marks of the current run (single-threaded use). */
    static final Map<String, Long> MARKS = new LinkedHashMap<>();
    static double simEnd = -1;

    public PatchedSimuComJob(SimuComWorkflowConfiguration configuration) throws CoreException {
        super(configuration, null, true);
    }

    @Override
    protected void addSimulatorSpecificJobs(SimuComWorkflowConfiguration configuration) {
        this.add(new Mark("prepared")); // project created, models loaded, validated, stored
        this.add(new DetermineFailureTypesJob(configuration));
        this.addJob(new XtendTransformPCMToCodeJob(configuration));
        // generated MANIFEST.MF additionally requires de.uka.ipd.sdq.errorhandling.core (see SourcePatches)
        this.addJob(new CreateSimuComMetaDataFilesJob(configuration) {
            @Override
            protected String[] getRequiredBundles() {
                String[] b = super.getRequiredBundles();
                String[] r = java.util.Arrays.copyOf(b, b.length + SourcePatches.EXTRA_BUNDLES.length);
                System.arraycopy(SourcePatches.EXTRA_BUNDLES, 0, r, b.length, SourcePatches.EXTRA_BUNDLES.length);
                return r;
            }
        });
        this.add(new FixGeneratedSourcesJob(configuration.getStoragePluginID()));
        this.add(new Mark("codegen"));
        this.addJob(new CompilePluginCodeJob(configuration));
        this.add(new Mark("compile"));
        BuildPluginJarJob jar = new BuildPluginJarJob(configuration);
        this.addJob(jar);
        this.add(new Mark("jar"));
        this.add(new DockJob(configuration, jar));
    }

    static final class Mark implements IJob {
        private final String name;

        Mark(String name) {
            this.name = name;
        }

        @Override
        public void execute(IProgressMonitor m) {
            MARKS.put(name, System.nanoTime());
        }

        @Override
        public void cleanup(IProgressMonitor m) {
        }

        @Override
        public String getName() {
            return "mark " + name;
        }
    }

    static final class DockJob implements IJob {
        private final SimuComWorkflowConfiguration cfg;
        private final BuildPluginJarJob jar;

        DockJob(SimuComWorkflowConfiguration cfg, BuildPluginJarJob jar) {
            this.cfg = cfg;
            this.jar = jar;
        }

        @Override
        public void execute(IProgressMonitor monitor) throws JobFailedException, UserCanceledException {
            try {
                DockModel dock = SimuControllerPlugin.getDockModel().getBestFreeDock();
                SimulationDockService svc = dock.getService();
                svc.load(cfg.getSimulationConfiguration(), jar.getResult(), dock.isRemote());
                SimuComModel model = svc instanceof SimulationDockServiceImpl
                        ? ((SimulationDockServiceImpl) svc).getSimuComModel() : null;
                MARKS.put("install", System.nanoTime());
                svc.simulate(cfg.getSimulationConfiguration(), jar.getResult(), dock.isRemote());
                MARKS.put("simulate", System.nanoTime());
                simEnd = model != null ? model.getSimulationControl().getCurrentSimulationTime() : -1;
            } catch (InterruptedException e) {
                throw new JobFailedException("Job failed while waiting for a dock to become available", e);
            } catch (Exception e) {
                throw new JobFailedException("Simulation run failed.", e);
            }
        }

        @Override
        public void cleanup(IProgressMonitor m) throws CleanupFailedException {
        }

        @Override
        public String getName() {
            return "Install simulation bundle in dock and simulate";
        }
    }
}
